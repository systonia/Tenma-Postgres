//! The native half of tenma/postgres: starts one Postgres server per process from the binaries next
//! to the package, with its data in a directory of the app, and stops it when tenma ends.
//!
//! The server listens on 127.0.0.1 only, on a free port, with a password made at the first start.
//! Unix sockets stay off: their path may have at most 103 bytes, and the data directory of an app can
//! be deeper than that.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tenma_plugin::{LOG_INFO, LOG_WARNING, Plugin, Reply, api};

static PLUGIN: Plugin = Plugin {
    name: c"postgres".as_ptr(),
    call: Some(call),
    call_async: None,
    adopt: None,
    message: None,
    closed: None,
    shutdown: Some(shutdown),
};

/// The running server, once a worker started it; the other workers get the same answer.
static SERVER: Mutex<Option<Running>> = Mutex::new(None);

const USER: &str = "tenma";
const DATABASE: &str = "postgres";
/// Next to the cluster's own files, readable only by the owner like the rest of the directory.
const PASSWORD_FILE: &str = "tenma-password";

struct Running {
    binaries: PathBuf,
    data: PathBuf,
    server: Server,
}

#[derive(Deserialize)]
struct Start {
    binaries: PathBuf,
    data: PathBuf,
}

#[derive(Serialize, Clone)]
struct Server {
    host: &'static str,
    port: u16,
    user: &'static str,
    password: String,
    database: &'static str,
}

#[unsafe(no_mangle)]
extern "C" fn tenma_plugin_init(api: &'static tenma_plugin::Api, abi: u32) -> *const Plugin {
    if abi != tenma_plugin::ABI {
        return std::ptr::null();
    }
    tenma_plugin::set_api(api);
    &PLUGIN
}

extern "C" fn call(
    function: *const std::ffi::c_char,
    function_len: usize,
    input: *const u8,
    input_len: usize,
    out: *mut Reply,
) -> i32 {
    let (function, input) = unsafe {
        (
            tenma_plugin::slice(function.cast(), function_len),
            tenma_plugin::slice(input, input_len),
        )
    };
    let result = match function {
        b"start" => start(input),
        _ => Err(format!("unknown function {}", String::from_utf8_lossy(function))),
    };
    match result {
        Ok(reply) => {
            api().reply(out, reply.as_bytes());
            0
        }
        Err(message) => {
            api().reply(out, message.as_bytes());
            1
        }
    }
}

/// Creates the cluster on the first start, then starts the server, or finds the one a previous
/// process left running (a restart in watch mode, a crash) and uses it.
fn start(input: &[u8]) -> Result<String, String> {
    let start: Start = serde_json::from_slice(input).map_err(|error| error.to_string())?;
    let mut running = SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(running) = running.as_ref() {
        if running.data != start.data {
            return Err(format!(
                "postgres: already running for {}, one data directory per process",
                running.data.display()
            ));
        }
        return to_json(&running.server);
    }

    let binaries = start.binaries;
    if !binaries.join("bin/pg_ctl").is_file() {
        return Err(format!(
            "postgres: no binaries in {}; run vendor/bin/tenma-postgres install",
            binaries.display()
        ));
    }
    let data = absolute(&start.data)?;
    if !data.join("PG_VERSION").is_file() {
        create(&binaries, &data)?;
    }
    let password = fs::read_to_string(data.join(PASSWORD_FILE))
        .map_err(|error| format!("postgres: {PASSWORD_FILE}: {error}"))?;

    let port = match port_in_use(&binaries, &data) {
        Some(port) => port,
        None => launch(&binaries, &data)?,
    };
    let server = Server {
        host: "127.0.0.1",
        port,
        user: USER,
        password: password.trim().to_owned(),
        database: DATABASE,
    };
    let json = to_json(&server)?;
    *running = Some(Running { binaries, data, server });
    Ok(json)
}

/// `initdb` with a fresh password, so only this app can log in, even from another local user.
fn create(binaries: &Path, data: &Path) -> Result<(), String> {
    api().log(LOG_INFO, &format!("postgres: creating {}", data.display()));
    let password = random_password()?;
    // initdb wants the password in a file, and the data directory must be empty for it, so the file
    // lies next to it while initdb runs. A file left by a crash goes first; create_new refuses a
    // file or symlink that is there anyway, and mode keeps the file private from the start
    let mut file = data.as_os_str().to_owned();
    file.push(".initdb-password");
    let file = PathBuf::from(file);
    let _ = fs::remove_file(&file);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&file)
        .and_then(|mut handle| handle.write_all(password.as_bytes()))
        .map_err(|error| format!("postgres: {}: {error}", file.display()))?;

    let result = run(Command::new(binaries.join("bin/initdb"))
        .arg("--pgdata")
        .arg(data)
        .args(["--username", USER, "--auth", "scram-sha-256", "--encoding", "UTF8"])
        // The builtin provider needs no locales from the system, so a cluster behaves the same everywhere
        .args(["--locale-provider", "builtin", "--locale", "C.UTF-8"])
        .arg("--pwfile")
        .arg(&file));
    let _ = fs::remove_file(&file);
    result?;

    let target = data.join(PASSWORD_FILE);
    fs::write(&target, &password).map_err(|error| error.to_string())?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())
}

/// Starts the server on a free port and waits until it accepts connections.
fn launch(binaries: &Path, data: &Path) -> Result<u16, String> {
    // ponytail: another process may take the port between here and the start; the start fails then
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let options = format!(
        "-p {port} -c listen_addresses=127.0.0.1 -c unix_socket_directories=''"
    );
    run(Command::new(binaries.join("bin/pg_ctl"))
        .arg("--pgdata")
        .arg(data)
        .arg("--log")
        .arg(data.join("server.log"))
        .args(["--options", &options, "--wait", "--timeout", "60", "start"]))?;
    api().log(LOG_INFO, &format!("postgres: listening on 127.0.0.1:{port}"));
    Ok(port)
}

/// The port of a server already running on this data directory, from line 4 of postmaster.pid.
fn port_in_use(binaries: &Path, data: &Path) -> Option<u16> {
    let status = Command::new(binaries.join("bin/pg_ctl"))
        .arg("--pgdata")
        .arg(data)
        .arg("status")
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let pid = fs::read_to_string(data.join("postmaster.pid")).ok()?;
    pid.lines().nth(3)?.trim().parse().ok()
}

/// Stops the server when tenma ends; the running queries get cancelled (`fast`), the data is safe.
extern "C" fn shutdown() {
    let running = SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(running) = running.as_ref() else {
        return;
    };
    let stopped = run(Command::new(running.binaries.join("bin/pg_ctl"))
        .arg("--pgdata")
        .arg(&running.data)
        .args(["--mode", "fast", "--wait", "--timeout", "30", "stop"]));
    if let Err(message) = stopped {
        api().log(LOG_WARNING, &message);
    }
}

fn run(command: &mut Command) -> Result<(), String> {
    let name = command.get_program().to_string_lossy().into_owned();
    // The server outlives the command and would keep every descriptor tenma and PHP left open
    // without close-on-exec, the pipes to a parent process among them, which then waits forever
    unsafe { command.pre_exec(close_inherited) };
    let output = command.output().map_err(|error| format!("postgres: {name}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "postgres: {name} failed: {}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    ))
}

/// Marks every descriptor above stderr close-on-exec, in the child between fork and exec.
fn close_inherited() -> std::io::Result<()> {
    let limit = unsafe { libc::getdtablesize() };
    for descriptor in 3..limit {
        unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok(())
}

/// The data directory as an absolute path, created with its parents if needed.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|error| format!("postgres: {}: {error}", path.display()))?;
    fs::canonicalize(path).map_err(|error| format!("postgres: {}: {error}", path.display()))
}

/// 32 hex characters from the system's random source.
fn random_password() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn to_json(server: &Server) -> Result<String, String> {
    serde_json::to_string(server).map_err(|error| error.to_string())
}
