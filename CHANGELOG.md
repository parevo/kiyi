# Changelog

What changed in each version of Kiyi, newest first. The [releases page](https://github.com/parevo/kiyi/releases) has the downloads.

## 0.6.4 (2026-10-28)

### New
- **Back up and restore show how far they are, and can be stopped.** A bar with the table being read, rows and size written so far, elapsed time, and a Stop button. Stopping ends the read on the server (or the pg_dump / mysqldump / psql / mysql / sqlcmd run), and a restore made from a Kiyi backup is rolled back. The confirmation for a restore now closes right away, and progress shows in the view.

### Fixed
- **SQL Server:** a backup, or any query, that waited more than 30 seconds for the server (a table locked by someone else, a slow scan) no longer fails with "the server did not send the next part of the command result". Long queries can still be stopped as before.
- **Backups:** a stopped or failed backup leaves no half-written file, and a file already at that path is kept as it was. The backup is moved into place only when it's complete.
- **SQL Server backups** no longer say PostgreSQL in their first line.

## 0.6.3 (2026-10-28)

### Fixed
- **Move data, existing rows:** "Skip" and "Update" now ask the target whether a row is already there, so its own rules for case, accents and trailing spaces decide, as its unique index will. Before, a case-sensitive database (PostgreSQL, SQLite) could pass the check and then stop the move on a clashing key.
- **Move data, repeated rows:** source rows with the same match values (two customers with one email) are moved once, and links to the second follow the first. The check points them out and the report counts them.
- **Move data from SQL Server:** tables without a key that have text, ntext, image or xml columns can be read, tables with a key of several columns are read fast at any size, and very large tables are counted correctly.
- **Move data into MySQL:** BIT columns get 1 or 0 instead of failing, uniqueidentifiers go into BINARY(16) as 16 bytes, and values that don't fit (a tinyint over 127, too many digits for a DECIMAL, text over 64 KB for TEXT, a TIMESTAMP outside 1970–2038) are shown by the check instead of stopping the move.
- **Move data:** the temporary file a move writes is readable only by you, and one left behind by a crash is removed. A plan saved by a newer Kiyi says so instead of opening wrongly.

## 0.6.2 (2026-10-25)

### New
- **A connection window that shows the way.** Connecting is now a few numbered steps for each way in (direct, SSH, AWS SSM, Kubernetes, Cloud SQL), and the step you're on says where to find each value, with the path in the AWS console: the EC2 address and user, the .pem key, the RDS endpoint and master user. Paste the ssh command AWS shows under Connect (ssh -i key.pem ec2-user@…) and the SSH step fills itself in. Values that look like they're in the wrong place are pointed out, such as an RDS endpoint entered as the SSH server or a .pub key. Encryption and AWS IAM sign-in fold under More options until you need them.
- **AI setup that checks itself.** Settings → AI tries each provider when it opens and says in words what's wrong, with a Fix it button. A key pasted under the wrong company is recognised by how it starts (xai-, gsk_, sk-ant-…) and Kiyi offers to switch, keeping the key. Setting up is three steps: open the provider's key page, paste the key (its models are listed right away), choose a model.

### Fixed
- **AI:** a model the provider doesn't offer, such as a Grok model under Groq, can no longer be saved, so AI no longer fails on every request after such a setup.

## 0.6.0 (2026-10-12)

### New
- **Data quality.** A Quality tab on every table, and Check data on the Overview for the whole database, point out what's messy: duplicate and copied rows, one value spelled several ways ("Istanbul", "istanbul ", "İstanbul"), stray spaces, empty text, invalid emails, outliers, negative amounts, dates in the future or far past, columns that are never filled. Most come with a fix you review before it runs.
- **Dashboards.** Add any chart from the SQL editor to a dashboard and keep several side by side. Each chart re-runs its query when the dashboard opens or on Refresh (or every few minutes, if you choose). Dashboards travel with exported connections.
- **Edit query results.** When a query reads one table and shows its key (`SELECT id, name, email FROM customers WHERE …`), its results can be edited like the table itself: change cells, then Save. Joins, groups and computed columns stay read-only, and production saves are reviewed first.

### Fixed
- A change made just before the window closed or reloaded is no longer lost: it reaches Kiyi's folder the next time Kiyi opens.

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
