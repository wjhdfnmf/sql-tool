//! Working out what kind of server is on the other end.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Flavor {
    MySql,
    MariaDb,
    Percona,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServerVersion {
    pub flavor: Flavor,
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    /// Oracle's commercial build (MySQL Enterprise).
    pub enterprise: bool,
    /// The version string exactly as the server reported it.
    pub raw: String,
}

impl ServerVersion {
    /// `version` is `@@version`; `comment` is `@@version_comment`.
    pub fn parse(version: &str, comment: &str) -> Self {
        let lower = version.to_ascii_lowercase();
        let comment_lower = comment.to_ascii_lowercase();

        let flavor = if lower.contains("mariadb") {
            Flavor::MariaDb
        } else if comment_lower.contains("percona") {
            Flavor::Percona
        } else {
            Flavor::MySql
        };

        // MariaDB before 11 prefixes its real version with "5.5.5-" so that
        // old replication clients accept it.
        let numeric = match flavor {
            Flavor::MariaDb => version.strip_prefix("5.5.5-").unwrap_or(version),
            _ => version,
        };
        let mut parts = numeric
            .split(|c: char| !c.is_ascii_digit())
            .take(3)
            .map(|p| p.parse::<u16>().unwrap_or(0));
        let major = parts.next().unwrap_or(0);
        let minor = parts.next().unwrap_or(0);
        let patch = parts.next().unwrap_or(0);

        let enterprise = flavor == Flavor::MySql
            && (lower.contains("enterprise")
                || lower.contains("commercial")
                || comment_lower.contains("enterprise"));

        Self {
            flavor,
            major,
            minor,
            patch,
            enterprise,
            raw: version.to_string(),
        }
    }

    pub fn at_least(&self, major: u16, minor: u16, patch: u16) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(version: &str, comment: &str) -> (Flavor, u16, u16, u16, bool) {
        let p = ServerVersion::parse(version, comment);
        (p.flavor, p.major, p.minor, p.patch, p.enterprise)
    }

    #[test]
    fn mysql_enterprise_8_4() {
        assert_eq!(
            v("8.4.3-commercial", "MySQL Enterprise Server - Commercial"),
            (Flavor::MySql, 8, 4, 3, true)
        );
    }

    #[test]
    fn mysql_enterprise_5_7() {
        assert_eq!(
            v(
                "5.7.37-enterprise-commercial-advanced-log",
                "MySQL Enterprise Server - Advanced Edition (Commercial)"
            ),
            (Flavor::MySql, 5, 7, 37, true)
        );
    }

    #[test]
    fn mysql_community() {
        assert_eq!(
            v("8.0.40", "MySQL Community Server - GPL"),
            (Flavor::MySql, 8, 0, 40, false)
        );
    }

    #[test]
    fn mariadb_with_replication_prefix() {
        assert_eq!(
            v("5.5.5-10.6.12-MariaDB-log", "mariadb.org binary distribution"),
            (Flavor::MariaDb, 10, 6, 12, false)
        );
    }

    #[test]
    fn mariadb_without_prefix() {
        assert_eq!(
            v("11.4.2-MariaDB-ubu2404", "mariadb.org binary distribution"),
            (Flavor::MariaDb, 11, 4, 2, false)
        );
    }

    #[test]
    fn percona() {
        assert_eq!(
            v("8.0.36-28", "Percona Server (GPL), Release 28, Revision 47601f19"),
            (Flavor::Percona, 8, 0, 36, false)
        );
    }

    #[test]
    fn at_least_compares_in_order() {
        let p = ServerVersion::parse("8.4.3-commercial", "");
        assert!(p.at_least(8, 0, 16));
        assert!(p.at_least(8, 4, 3));
        assert!(!p.at_least(9, 0, 0));
    }
}
