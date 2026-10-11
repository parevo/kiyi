# Changelog

What changed in each version of Kiyi, newest first. The [releases page](https://github.com/parevo/kiyi/releases) has the downloads.

## 0.5.0 (2026-10-11)

### New
- **Query parameters.** Write `:customer_id` in a query and Kiyi asks for its value when it runs: numbers go in as numbers, anything else as safely quoted text. Values are remembered while the tab is open, so saved queries become reusable reports.
- **Hosted databases.** Supabase, Neon, PlanetScale, Amazon RDS, Azure SQL, Azure Database, DigitalOcean, Railway, Render and Aiven are recognised from their address: SSL is turned on where they require it, and the connection window says where to find the details in their dashboard. The welcome screen has a shortcut for each.
- **Your work is kept safe.** Query history, saved queries, table views and column layouts, open query tabs and settings are now kept as files in Kiyi's folder, not only in the app's web storage, so clearing it loses nothing. Existing data moves over on first launch.
- **Saved queries and views travel with connections.** Exporting connections includes their saved queries and table views, and importing maps them to the connections on the new computer. Query history stays behind.
- **History can be turned off.** Settings → General → Query history, for queries that hold sensitive values.

### Fixed
- In narrow windows, the table toolbar, Move data and the Overview wrap onto a second line instead of pushing buttons (Insert row, Columns, Views, Save plan) out of reach.
- On the welcome screen, the list of databases found on this computer no longer collapses when the window is short.
- Tables created, renamed or dropped in the SQL editor appear in the sidebar right away.
- SQL autocomplete suggests only words that start with what you typed, not distant matches like USER_DEFINED_TYPE_SCHEMA for "desc".
- Objects no longer lists MySQL's own internal accounts (mysql.sys, mysql.session) among users.
- Every field in the row panel has an accessible name for screen readers.
- "1 rows" now says "1 row".

## 0.4.1 (2026-10-11)

### New
- **Linux.** Kiyi for 64-bit Linux, as an AppImage that updates itself and a .deb for Debian and Ubuntu. Passwords and keys are kept in the desktop's Secret Service (GNOME Keyring, KWallet).

### Fixed
- **Move data:** rows that point at other rows of the same table (a manager, a parent category) move correctly whatever their order: the links are written once every row exists. A link that can't be empty is pointed out by Check.

## 0.4.0 (2026-10-10)

### New
- **Move data between databases.** Bring another system's data into yours, even when the tables look nothing alike: PostgreSQL, MySQL, MariaDB, SQL Server and SQLite, in any direction. From the Overview, choose **Move data**.
  - Kiyi suggests a plan from table and column names (including common synonyms in English and Turkish), types and foreign keys, and splits or joins names where one side has first and last names and the other a full name. **Improve with AI** refines it; it sees only table and column names unless you allow three example rows.
  - Each column gets its value from a source column, several joined, a fixed value, or the new ID of a moved row, cleaned up by readable steps: trim, change case, replace, take part of the text, change values ("A" → "active"), or fill in when empty.
  - IDs can be kept or renumbered after the highest one already there; links between moved tables follow automatically, including links to rows that already existed.
  - When a row already exists: stop, skip it, or update it, matched on its key or any unique column.
  - **Check** reads a sample and shows the rows as they'll be stored, and every problem by column. **Test run** writes everything in a transaction and rolls it back, so the database itself confirms every row. **Move data** runs in one transaction with progress and a Stop button: it all lands, or nothing does.
  - Values are converted between databases: time zones to UTC where the target has none, fractions of a second cut to the column's precision, yes/no in each database's form, Postgres arrays to JSON, binary, numbers written with commas, lengths and allowed values checked.
  - Plans can be saved to a file and opened again, on another computer too.

## 0.3.0 (2026-10-10)

### New
- **Objects.** Functions, procedures, triggers, sequences, and users and roles, next to the schema diagram. See the SQL that defines each one, open it in the SQL editor, drop it, or start a new one from a template. Changes go through the SQL editor, so production connections get their usual review.
- **Sample database.** Try Kiyi without a database of your own: "Try the sample database" on the welcome screen (or ⌘K) opens a small store with customers, products and orders.
- **Move connections between computers.** Export connections to a file and import them elsewhere. Passwords and keys stay in the keychain and are never written to the file; read-only and production settings travel along.
- **A closer look at values.** JSON opens as a tree you can fold, binary values show their size, what they probably are, an image preview and a hex dump.
- **Report a problem.** From Settings, ⌘K or the error screen, Kiyi opens a GitHub issue in your browser with your version, system and the end of the log filled in, to read before sending. After an unexpected exit, Kiyi offers to report it on the next launch.

### Fixed
- Running the statement under the cursor (⌘↵) no longer cuts MySQL and SQL Server procedures, functions and SQLite triggers apart at the semicolons inside their BEGIN … END.
- On SQL Server, CREATE PROCEDURE with parameters, and procedures, functions, triggers and views created during structure changes, failed with a syntax error.
- The Developer mode description no longer says it adds the SQL editor, which is always available.

## 0.2.0 (2026-10-10)

### New
- **SQL Server** 2017 and later, and Azure SQL: browsing, editing, structure changes, explain, backups and compare.
- **Ask a question, get a chart.** Ask in any language on the Overview or with ⌘K; Kiyi writes one read-only query, runs it and draws the answer.
- **Charts and summaries**: bars, lines, pies or a single figure, and a Summary tab that groups a table by any column.
- **SQL editor**: run, format, readable query plans, history, saved queries, and AI that writes, explains and fixes SQL.
- **Schema diagram, backup and restore, and database comparison.**
- **Excel** import and export, and export of query results.
- **Grid tools**: hidden, reordered and frozen columns, saved views, totals of selected cells, copy as CSV, Markdown, JSON or INSERTs, bulk edits, find and replace, and ⌘K.
- More ways to connect: hosts from `~/.ssh/config`, jump hosts, Kubernetes, the Cloud SQL Auth Proxy, RDS IAM sign-in and Unix sockets.

## 0.1.2 (2026-10-05)

- New app icon.

## 0.1.1 (2026-10-05)

- The macOS app and disk image are signed with a Developer ID and notarized by Apple.
- Faster sorted pages on MySQL.

## 0.1.0 (2026-10-05)

The first release: PostgreSQL, MySQL, MariaDB and SQLite; editing like a spreadsheet with Undo; filters in plain words with any AI provider; table design without DDL; SSH tunnels and AWS SSM; CSV and JSON import and export; production connections that are read-only by default.
