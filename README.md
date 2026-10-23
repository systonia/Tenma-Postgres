# tenma/postgres

A Postgres server of the app's own on Tenma. The app starts it next to itself, with its data in a directory the
app chooses, and Tenma stops it when the app ends. Meant for apps that ship as one download and need a database
without asking the user to set one up.

```php
use Tenma\Postgres\Server;

$server = Server::start(__DIR__ . '/data');
$pdo = new PDO($server->dsn());
```

- The first start creates the cluster (UTF-8, builtin locale) with a user `tenma` and a random password, kept in
  `data/tenma-password` (mode 0600).
- The server listens on 127.0.0.1 only, on a free port. Unix sockets stay off.
- It runs once per process: every worker gets the same server. A server that an earlier process left running on
  the same directory (a restart in watch mode, a crash) is found and used.
- It stops when tenma ends: after Ctrl+C or SIGTERM, and when a script that serves nothing finishes.
- `$server->url()` gives `postgresql://tenma:...@127.0.0.1:PORT/postgres`, the form of `DATABASE_URL`.

## Optional by design

The package is meant to be easy to add and to drop. An app asks for its own server only when no database is
configured, so the same code runs with an external Postgres and without:

```php
$url = getenv('DATABASE_URL') ?: Server::start($home . '/data')->url();
```

Without the package, that line becomes the plain `DATABASE_URL`, and nothing else changes.

## Install

```bash
composer require tenma/postgres
vendor/bin/tenma-postgres install
```

The install downloads the plugin of this version for the machine and Postgres 18 from
[theseus-rs/postgresql-binaries](https://github.com/theseus-rs/postgresql-binaries), checks both against their
SHA-256, keeps only the server, `initdb`, `pg_ctl` and the libraries they load (about 70 MB, 16 MB compressed)
in `vendor/tenma/postgres/postgres/`, and adds the plugin to `tenma.json`.

Platforms: macOS arm64, Linux x64 and arm64.

## Develop

```bash
cd plugin && cargo build --release && mkdir -p ../lib && cp target/release/libtenma_postgres.* ../lib/
cd .. && bin/tenma-postgres install
composer test
```

The tests run the plugin under a real tenma and skip when tenma, the plugin or Postgres is missing.

## License

The package: MIT or Apache-2.0, at your option. Postgres itself: the PostgreSQL License; the install copies it to
`postgres/LICENSE` and the notices of the bundled libraries to `postgres/dependency-notices/`.
