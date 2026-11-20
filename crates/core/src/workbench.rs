//! Reading the connections saved by MySQL Workbench, to offer them for import.
//!
//! Workbench keeps its connections in `connections.xml` in its data folder.
//! Each is an object with a driver (how it connects), a name, and a
//! dictionary of settings. This reads that file and turns each connection
//! into a profile of ours where it can, saying what does not carry over.
//!
//! Passwords are not in that file (Workbench keeps them in the operating
//! system's vault) and are not looked for anywhere else: an imported
//! connection asks for its password the first time it is used.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::Error;
use crate::profiles::{SavedProfile, TlsSetting};

/// A larger file than this is not a list of connections.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const DRIVER_PREFIX: &str = "com.mysql.rdbms.mysql.driver.";

/// One connection found in Workbench's file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub name: String,
    /// How Workbench connects with it, in Workbench's own words.
    pub method: String,
    /// What it would be here. `None` if it cannot be brought over.
    pub profile: Option<SavedProfile>,
    /// Why it cannot be brought over.
    pub problem: Option<String>,
    /// What will be different here, for a connection that can be.
    pub notes: Vec<String>,
}

/// Where Workbench keeps its connections on this system, whether or not the file is there.
pub fn default_path() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let folder = if cfg!(windows) {
        var("APPDATA")?.join("MySQL").join("Workbench")
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library").join("Application Support").join("MySQL").join("Workbench")
    } else {
        var("HOME")?.join(".mysql").join("workbench")
    };
    Some(folder.join("connections.xml"))
}

/// Reads the connections in the text of a `connections.xml`.
pub fn parse(xml: &str) -> Result<Vec<Found>, Error> {
    // A document type declaration is refused (the reader's default): the file has none, and
    // refusing it rules out the entity tricks a crafted file could play.
    let document = roxmltree::Document::parse(xml)
        .map_err(|e| Error::Other(format!("That file could not be read as a MySQL Workbench connections file: {e}")))?;
    if !document.root_element().has_tag_name("data") {
        return Err(Error::Other("That file is not a MySQL Workbench connections file.".into()));
    }
    let found = document
        .descendants()
        .filter(|node| node.has_tag_name("value") && node.attribute("struct-name") == Some("db.mgmt.Connection"))
        .map(|node| {
            let child = |tag: &str, key: &str| node.children().find(|c| c.has_tag_name(tag) && c.attribute("key") == Some(key));
            let text = |n: roxmltree::Node<'_, '_>| n.text().unwrap_or("").trim().to_string();
            let settings: HashMap<String, String> = child("value", "parameterValues")
                .map(|dict| dict.children().filter_map(|v| Some((v.attribute("key")?.to_string(), text(v)))).collect())
                .unwrap_or_default();
            convert(&child("value", "name").map(text).unwrap_or_default(), &child("link", "driver").map(text).unwrap_or_default(), &settings)
        })
        .collect();
    Ok(found)
}

fn convert(name: &str, driver: &str, settings: &HashMap<String, String>) -> Found {
    let get = |key: &str| settings.get(key).map(String::as_str).filter(|v| !v.is_empty());
    let host = get("hostName").unwrap_or("127.0.0.1").to_string();
    let name = if name.is_empty() { host.clone() } else { name.to_string() };
    let kind = driver.strip_prefix(DRIVER_PREFIX).unwrap_or(driver);
    let cannot = |method: &str, problem: &str| Found {
        name: name.clone(),
        method: method.to_string(),
        profile: None,
        problem: Some(problem.to_string()),
        notes: Vec::new(),
    };
    // Workbench's LDAP connection sends the password in clear text, as its cleartext option does.
    let (method, ldap) = match kind {
        "native" => ("Standard (TCP/IP)", false),
        "ldap_user_password" => ("LDAP User/Password", true),
        "native_socket" => return cannot("Local Socket/Pipe", "It connects through a local socket or pipe, which this app does not do."),
        "native_sshtun" => return cannot("Standard TCP/IP over SSH", "It connects through an SSH tunnel, which this app does not do."),
        "ldap_sasl_kerberos" => return cannot("LDAP Sasl/Kerberos", "It signs in with SASL or Kerberos, which this app does not do."),
        "ldap_kerberos" => return cannot("Native Kerberos", "It signs in with Kerberos, which this app does not do."),
        _ => return cannot("Unknown", "It connects in a way this app does not know."),
    };
    let Ok(port) = get("port").unwrap_or("3306").parse::<u16>() else {
        return cannot(method, "Its port is not a number from 1 to 65535.");
    };

    let mut notes = Vec::new();
    // Workbench numbers its SSL choices differently for the LDAP connection, which has no "No" or "If available".
    let ssl = get("useSSL").and_then(|v| v.parse::<i64>().ok());
    let tls = match (ldap, ssl) {
        (false, Some(0)) => TlsSetting::Disabled,
        (false, Some(1) | None) => {
            notes.push(
                "Workbench encrypted it only if the server offered to. Here it is always encrypted, with the server not verified; \
                 it will not connect to a server that cannot encrypt."
                    .to_string(),
            );
            TlsSetting::Unverified
        }
        (false, Some(2)) | (true, Some(0) | None) => TlsSetting::Unverified,
        (false, Some(3)) | (true, Some(1)) => TlsSetting::VerifyCa,
        (false, Some(4)) | (true, Some(2)) => TlsSetting::VerifyIdentity,
        _ => {
            notes.push("Its SSL setting was not recognised. It is set to verify the server, the safe choice; change it if that does not connect.".to_string());
            TlsSetting::VerifyIdentity
        }
    };
    let verifies = matches!(tls, TlsSetting::VerifyCa | TlsSetting::VerifyIdentity);
    let cleartext = ldap || get("OPT_ENABLE_CLEARTEXT_PLUGIN").is_some_and(|v| v != "0");
    if cleartext && !verifies {
        notes.push(
            "It sends the password in clear text to a server that is not verified. Before it connects you will be asked to accept \
             that in its settings; verifying the server, or pinning its certificate, is safer."
                .to_string(),
        );
    }
    if get("sslCert").is_some() || get("sslKey").is_some() {
        notes.push("It uses a client certificate, which this app does not send. A server that insists on one will refuse it.".to_string());
    }
    if get("$others").is_some() || get("SQL_MODE").is_some() {
        notes.push("Its extra options (SQL mode, other settings) are not carried over.".to_string());
    }
    Found {
        name: name.clone(),
        method: method.to_string(),
        profile: Some(SavedProfile {
            name,
            host,
            port,
            user: get("userName").unwrap_or("root").to_string(),
            database: get("schema").map(str::to_string),
            tls,
            ca_file: get("sslCA").filter(|_| verifies).map(PathBuf::from),
            cleartext_auth: cleartext,
            // Never accepted on the user's behalf.
            cleartext_risk_accepted: false,
            // The password is not in the file; it is asked for when the connection is first used.
            save_password: false,
            ..SavedProfile::default()
        }),
        problem: None,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(name: &str, driver: &str, settings: &[(&str, &str, &str)]) -> String {
        let values: String = settings.iter().map(|(kind, key, value)| format!("<value type=\"{kind}\" key=\"{key}\">{value}</value>\n")).collect();
        format!(
            r#"<value type="object" struct-name="db.mgmt.Connection" id="{{A1}}" struct-checksum="0x96ba47d8">
  <link type="object" struct-name="db.mgmt.Driver" key="driver">com.mysql.rdbms.mysql.driver.{driver}</link>
  <value type="string" key="hostIdentifier">Mysql@x:3306</value>
  <value type="int" key="isDefault">1</value>
  <value _ptr_="0x1" type="dict" key="modules"/>
  <value _ptr_="0x2" type="dict" key="parameterValues">
{values}  </value>
  <value type="string" key="name">{name}</value>
  <link type="object" struct-name="GrtObject" key="owner">{{B2}}</link>
</value>"#
        )
    }

    fn file(connections: &[String]) -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<data grt_format=\"2.0\">\n<value _ptr_=\"0x0\" type=\"list\" content-type=\"object\" content-struct-name=\"db.mgmt.Connection\">\n{}\n</value>\n</data>\n",
            connections.join("\n")
        )
    }

    fn one(driver: &str, settings: &[(&str, &str, &str)]) -> Found {
        parse(&file(&[connection("Line 3", driver, settings)])).unwrap().remove(0)
    }

    #[test]
    fn a_standard_connection_comes_over_with_its_host_port_user_and_schema() {
        let found = one(
            "native",
            &[("string", "hostName", "db.example.test"), ("int", "port", "3307"), ("string", "userName", "ann"), ("string", "schema", "plant"), ("int", "useSSL", "2"), ("string", "sslCA", "")],
        );
        assert_eq!((found.name.as_str(), found.method.as_str(), &found.problem), ("Line 3", "Standard (TCP/IP)", &None));
        let profile = found.profile.unwrap();
        assert_eq!((profile.host.as_str(), profile.port, profile.user.as_str()), ("db.example.test", 3307, "ann"));
        assert_eq!(profile.database.as_deref(), Some("plant"));
        assert_eq!(profile.tls, TlsSetting::Unverified);
        assert!(profile.id.is_empty() && !profile.save_password && !profile.cleartext_auth);
        assert!(found.notes.is_empty(), "{:?}", found.notes);
    }

    #[test]
    fn workbench_ssl_choices_become_ours() {
        let tls = |driver: &str, value: &str| one(driver, &[("int", "useSSL", value), ("string", "sslCA", "/etc/ca.pem")]).profile.unwrap();
        assert_eq!(tls("native", "0").tls, TlsSetting::Disabled);
        assert_eq!(tls("native", "1").tls, TlsSetting::Unverified);
        assert_eq!(tls("native", "2").tls, TlsSetting::Unverified);
        assert_eq!(tls("native", "3").tls, TlsSetting::VerifyCa);
        assert_eq!(tls("native", "4").tls, TlsSetting::VerifyIdentity);
        // The CA file is kept where the server is verified, and not where it would go unused.
        assert_eq!(tls("native", "3").ca_file, Some(PathBuf::from("/etc/ca.pem")));
        assert_eq!(tls("native", "2").ca_file, None);
        // "If available" has no equal here, and says so. No setting at all is Workbench's default, the same.
        assert!(one("native", &[("int", "useSSL", "1")]).notes[0].contains("only if the server offered"));
        assert!(one("native", &[]).notes[0].contains("only if the server offered"));
        // The LDAP connection numbers the same choices from nought.
        assert_eq!(tls("ldap_user_password", "0").tls, TlsSetting::Unverified);
        assert_eq!(tls("ldap_user_password", "1").tls, TlsSetting::VerifyCa);
        assert_eq!(tls("ldap_user_password", "2").tls, TlsSetting::VerifyIdentity);
        // A number Workbench does not use is not guessed at downwards.
        let odd = one("native", &[("int", "useSSL", "9")]);
        assert_eq!(odd.profile.unwrap().tls, TlsSetting::VerifyIdentity);
        assert!(odd.notes[0].contains("not recognised"));
    }

    #[test]
    fn clear_text_sign_in_is_carried_over_but_its_risk_is_never_accepted_for_the_user() {
        let found = one("native", &[("int", "useSSL", "2"), ("int", "OPT_ENABLE_CLEARTEXT_PLUGIN", "1")]);
        let profile = found.profile.unwrap();
        assert!(profile.cleartext_auth && !profile.cleartext_risk_accepted);
        assert!(found.notes.iter().any(|n| n.contains("clear text to a server that is not verified")), "{:?}", found.notes);
        // Verified, there is nothing to warn of.
        let verified = one("native", &[("int", "useSSL", "4"), ("int", "OPT_ENABLE_CLEARTEXT_PLUGIN", "1")]);
        assert!(verified.profile.unwrap().cleartext_auth && verified.notes.is_empty());
        // Off, or absent, is off.
        assert!(!one("native", &[("int", "useSSL", "2"), ("int", "OPT_ENABLE_CLEARTEXT_PLUGIN", "0")]).profile.unwrap().cleartext_auth);
        // Workbench's LDAP connection is clear text by its nature.
        assert!(one("ldap_user_password", &[("int", "useSSL", "2")]).profile.unwrap().cleartext_auth);
    }

    #[test]
    fn what_this_app_cannot_do_is_listed_with_the_reason_and_no_profile() {
        for (driver, reason) in [("native_socket", "socket or pipe"), ("native_sshtun", "SSH tunnel"), ("ldap_sasl_kerberos", "SASL or Kerberos"), ("ldap_kerberos", "Kerberos"), ("something_new", "does not know")] {
            let found = one(driver, &[("string", "hostName", "h"), ("string", "sshHost", "jump:22")]);
            assert!(found.profile.is_none(), "{driver}");
            assert!(found.problem.as_deref().unwrap().contains(reason), "{driver}: {:?}", found.problem);
        }
        assert!(one("native", &[("int", "port", "99999")]).problem.unwrap().contains("port"));
    }

    #[test]
    fn what_is_left_behind_is_said() {
        let found = one("native", &[("int", "useSSL", "4"), ("string", "sslCert", "/c.pem"), ("string", "sslKey", "/k.pem"), ("string", "SQL_MODE", "ANSI")]);
        assert!(found.notes.iter().any(|n| n.contains("client certificate")));
        assert!(found.notes.iter().any(|n| n.contains("extra options")));
    }

    #[test]
    fn defaults_fill_what_the_file_leaves_out_and_every_connection_is_found() {
        let bare = one("native", &[("int", "useSSL", "2")]).profile.unwrap();
        assert_eq!((bare.host.as_str(), bare.port, bare.user.as_str(), bare.database), ("127.0.0.1", 3306, "root", None));
        let all = parse(&file(&[
            connection("One", "native", &[("string", "hostName", "a")]),
            connection("", "native", &[("string", "hostName", "b.example.test")]),
            connection("Tunnel &amp; &lt;odd&gt; name", "native_sshtun", &[]),
        ]))
        .unwrap();
        let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
        // A connection with no name is called by its host; names come through as written.
        assert_eq!(names, ["One", "b.example.test", "Tunnel & <odd> name"]);
        assert!(parse(&file(&[])).unwrap().is_empty());
    }

    #[test]
    fn other_files_are_refused() {
        assert!(parse("not xml at all").unwrap_err().to_string().contains("could not be read"));
        assert!(parse("<html><body/></html>").unwrap_err().to_string().contains("not a MySQL Workbench"));
        // A document type declaration, which a connections file never has, is not processed.
        let crafted = "<?xml version=\"1.0\"?><!DOCTYPE data [<!ENTITY e \"x\">]><data>&e;</data>";
        assert!(parse(crafted).is_err());
    }

    #[test]
    fn the_file_is_looked_for_where_workbench_keeps_it() {
        let path = default_path().expect("a home folder");
        assert!(path.ends_with("connections.xml"));
        assert!(path.to_string_lossy().to_lowercase().contains("workbench"));
    }
}
