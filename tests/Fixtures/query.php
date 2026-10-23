<?php

// Run by ServerTest under tenma: starts the server with its data in $argv[1] and asks it one question.

require __DIR__ . '/../../vendor/autoload.php';

use Tenma\Postgres\Server;

$server = Server::start($argv[1] ?? '');
$pdo = new PDO($server->dsn());
$statement = $pdo->query('SELECT 1 + 1');
if ($statement === false) {
    exit(1);
}

echo $server->port, ' ', json_encode($statement->fetchColumn()), "\n";
