<?php

namespace Tenma\Postgres;

use Tenma\Plugin;

/**
 * A Postgres server of the app's own, started by libtenma_postgres (loaded with `tenma --plugin`). It runs
 * once per process, listens on 127.0.0.1 only and stops when tenma ends.
 */
final readonly class Server
{
    private const string PLUGIN = 'postgres';

    public function __construct(
        public string $host,
        public int $port,
        public string $user,
        public string $password,
        public string $database,
    ) {
    }

    /**
     * Starts the server with its data in $directory (created on the first start), or returns the one already
     * running. $binaries is where `vendor/bin/tenma-postgres install` put Postgres.
     */
    public static function start(string $directory, ?string $binaries = null): self
    {
        $input = json_encode(
            [
                'data' => $directory,
                'binaries' => $binaries ?? dirname(__DIR__) . '/postgres',
            ],
            JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES,
        );
        $answer = json_decode(Plugin::call(self::PLUGIN, 'start', $input), true, flags: JSON_THROW_ON_ERROR);
        if (!is_array($answer)) {
            throw new \UnexpectedValueException('postgres: the plugin answered no object');
        }

        return new self(
            host: self::text($answer, 'host'),
            port: (int) self::text($answer, 'port'),
            user: self::text($answer, 'user'),
            password: self::text($answer, 'password'),
            database: self::text($answer, 'database'),
        );
    }

    /** For `new PDO($server->dsn())`. */
    public function dsn(): string
    {
        return "pgsql:host=$this->host;port=$this->port;dbname=$this->database;"
            . "user=$this->user;password=$this->password";
    }

    /** postgresql://user:password@host:port/database, in the form of DATABASE_URL. */
    public function url(): string
    {
        return "postgresql://$this->user:$this->password@$this->host:$this->port/$this->database";
    }

    /** @param array<mixed> $answer */
    private static function text(array $answer, string $key): string
    {
        $value = $answer[$key] ?? null;
        if (!is_string($value) && !is_int($value)) {
            throw new \UnexpectedValueException("postgres: the plugin's answer has no $key");
        }

        return (string) $value;
    }
}
