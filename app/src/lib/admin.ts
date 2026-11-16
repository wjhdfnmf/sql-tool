// Server information shortcuts. Each one is an ordinary statement run in a
// query tab, so what it does is always visible; they are not dashboards.

export const adminQueries: { title: string; note: string; sql: string }[] = [
  { title: 'Server Status', note: 'Counters and gauges: uptime, threads, traffic, cache use', sql: 'SHOW GLOBAL STATUS' },
  { title: 'Client Connections', note: 'Who is connected and what each connection is running', sql: 'SHOW FULL PROCESSLIST' },
  { title: 'System Variables', note: 'How the server is configured', sql: 'SHOW GLOBAL VARIABLES' },
  { title: 'Users and Privileges', note: 'Accounts and how each one logs in', sql: 'SELECT user, host, plugin FROM mysql.user ORDER BY user, host' },
  { title: 'My Privileges', note: 'What this account is allowed to do', sql: 'SHOW GRANTS' },
  { title: 'Storage Engines', note: 'Which engines are available and the default', sql: 'SHOW ENGINES' },
];
