use anyhow::{bail, Context, Result};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    sync::Arc,
    time::Duration,
};
trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}
pub fn handle(connection: TcpStream, root: &Path, tls: Option<Arc<ServerConfig>>) -> Result<()> {
    connection.set_read_timeout(Some(Duration::from_secs(10)))?;
    connection.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut stream: Box<dyn Stream> = if let Some(tls) = tls {
        Box::new(StreamOwned::new(ServerConnection::new(tls)?, connection))
    } else {
        Box::new(connection)
    };
    let mut header = Vec::new();
    let mut byte = [0];
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 16384 {
            bail!("Fixture request header too large");
        }
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        header.push(byte[0]);
    }
    let text = std::str::from_utf8(&header)?;
    let mut words = text
        .lines()
        .next()
        .context("Missing request")?
        .split_whitespace();
    let method = words.next().context("Missing method")?;
    let request = words.next().context("Missing path")?;
    // Absolute-form requests exercise proxy selection without resolving the
    // deliberately nonexistent origin name. Nothing is forwarded externally.
    let target = if let Some((_, rest)) = request.split_once("://") {
        rest.find('/').map(|index| &rest[index..]).unwrap_or("/")
    } else {
        request
    };
    let path = target.split('?').next().unwrap_or("/");
    if path == "/eventually" {
        static REQUESTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let ready = REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2;
        let status = if ready { "200 OK" } else { "503 Unavailable" };
        let body = if ready {
            b"ready".as_slice()
        } else {
            b"pending".as_slice()
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        if method != "HEAD" {
            stream.write_all(body)?;
        }
        return Ok(());
    }
    if path == "/echo" || path == "/auth" {
        let headers = text
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect::<std::collections::BTreeMap<_, _>>();
        if path == "/auth" {
            let authenticated = headers
                .get("authorization")
                .is_some_and(|value| value == "Basic dXNlcjpwYXNz");
            if !authenticated {
                stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                return Ok(());
            }
        }
        let length = headers
            .get("content-length")
            .map(|value| value.parse::<usize>())
            .transpose()?
            .unwrap_or(0);
        if length > 8 * 1024 * 1024 {
            bail!("Fixture body too large");
        }
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        let body = serde_json::to_vec(
            &serde_json::json!({"method":method,"body":String::from_utf8_lossy(&body),"headers":headers}),
        )?;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
        if method != "HEAD" {
            stream.write_all(&body)?;
        }
        stream.flush()?;
        return Ok(());
    }
    if path == "/oversized" {
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 9437184\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    }
    if path == "/redirect" || path == "/redirect-loop" {
        let location = if path == "/redirect" {
            "/hello"
        } else {
            "/redirect-loop"
        };
        write!(stream,"HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }
    if path == "/status" {
        stream.write_all(
            b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    }
    if path == "/truncated" {
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 99999\r\nConnection: close\r\n\r\nshort",
        )?;
        return Ok(());
    }
    if path == "/slow" {
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 9999999\r\nConnection: close\r\n\r\npartial",
        )?;
        stream.flush()?;
        std::thread::sleep(Duration::from_secs(10));
        return Ok(());
    }
    if path == "/hello" || path == "/empty" {
        let body = if path == "/hello" {
            b"hello".as_slice()
        } else {
            b"".as_slice()
        };
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        if method != "HEAD" {
            stream.write_all(body)?;
        }
        stream.flush()?;
        return Ok(());
    }
    let name = path
        .strip_prefix('/')
        .context("Expected absolute fixture path")?;
    if Path::new(name).components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        bail!("Fixture path traversal rejected");
    }
    let mut file = match fs::File::open(root.join(name)) {
        Ok(file) if file.metadata()?.is_file() => file,
        _ => {
            stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )?;
            return Ok(());
        }
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        file.metadata()?.len()
    )?;
    if method != "HEAD" {
        std::io::copy(&mut file, &mut stream)?;
    }
    stream.flush()?;
    Ok(())
}
