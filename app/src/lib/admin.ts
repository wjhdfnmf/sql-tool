// Server administration shortcuts. Each one is an ordinary statement run in
// a query tab, so what it does is always visible; they are not dashboards.

export const adminQueries: { title: string; sql: string }[] = [
  { title: 'Server Status', sql: 'SHOW GLOBAL STATUS' },
  { title: 'Client Connections', sql: 'SHOW FULL PROCESSLIST' },
  { title: 'System Variables', sql: 'SHOW GLOBAL VARIABLES' },
  { title: 'Users and Privileges', sql: 'SELECT user, host, plugin FROM mysql.user ORDER BY user, host' },
  { title: 'My Privileges', sql: 'SHOW GRANTS' },
  { title: 'Storage Engines', sql: 'SHOW ENGINES' },
];
