<?php

namespace Tenma\Postgres\Tests;

use PHPUnit\Framework\TestCase;

/** End to end: the plugin under a real tenma with the installed Postgres; skipped when one of them is missing. */
final class ServerTest extends TestCase
{
    private string $data;

    protected function setUp(): void
    {
        if (!is_file(self::postgres() . '/bin/pg_ctl')) {
            self::markTestSkipped('No Postgres: run bin/tenma-postgres install');
        }
        if (self::plugin() === null) {
            self::markTestSkipped('No plugin in lib/: build it with cargo build --release in plugin/');
        }
        exec('command -v tenma', result_code: $status);
        if ($status !== 0) {
            self::markTestSkipped('No tenma on the PATH');
        }
        $this->data = sys_get_temp_dir() . '/tenma-postgres-test-' . bin2hex(random_bytes(4));
    }

    protected function tearDown(): void
    {
        if (!isset($this->data)) {
            return;
        }
        exec($this->pgCtl('--mode immediate stop'));
        exec('rm -rf ' . escapeshellarg($this->data));
    }

    public function testCreatesTheClusterAnswersAndStopsWithTenma(): void
    {
        [$port, $sum] = $this->query();

        self::assertSame('2', $sum);
        self::assertGreaterThan(0, (int) $port);
        self::assertSame(0600, fileperms($this->data . '/tenma-password') & 0777);
        exec($this->pgCtl('status'), result_code: $status);
        self::assertSame(3, $status, 'pg_ctl status: the server still runs');
    }

    public function testUsesTheServerAnEarlierProcessLeftRunning(): void
    {
        $this->query();
        // As after a restart in watch mode, which replaces the process without stopping the plugins
        $socket = stream_socket_server('tcp://127.0.0.1:0');
        self::assertNotFalse($socket);
        $port = (string) parse_url('tcp://' . stream_socket_get_name($socket, false), PHP_URL_PORT);
        fclose($socket);
        $options = escapeshellarg("-p $port -c listen_addresses=127.0.0.1 -c unix_socket_directories=''");
        $log = escapeshellarg($this->data . '/server.log');
        exec($this->pgCtl("--options $options --log $log --wait start"), $output, $status);
        self::assertSame(0, $status, implode("\n", $output));

        [$found, $sum] = $this->query();

        self::assertSame($port, $found);
        self::assertSame('2', $sum);
    }

    /** @return list<string> the port and the answer */
    private function query(): array
    {
        $command = 'tenma --plugin ' . escapeshellarg((string) self::plugin()) . ' '
            . escapeshellarg(__DIR__ . '/Fixtures/query.php') . ' ' . escapeshellarg($this->data) . ' 2>&1';
        exec($command, $output, $status);
        self::assertSame(0, $status, implode("\n", $output));

        return explode(' ', (string) end($output));
    }

    private function pgCtl(string $arguments): string
    {
        return escapeshellarg(self::postgres() . '/bin/pg_ctl') . ' --pgdata ' . escapeshellarg($this->data)
            . " $arguments 2>&1";
    }

    private static function postgres(): string
    {
        return dirname(__DIR__) . '/postgres';
    }

    private static function plugin(): ?string
    {
        return glob(dirname(__DIR__) . '/lib/libtenma_postgres.*')[0] ?? null;
    }
}
