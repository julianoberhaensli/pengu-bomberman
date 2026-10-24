use percent_encoding::percent_decode_str;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

const MAX_HEADERS: usize = 16384;

pub(crate) fn route(stream: &mut (impl Read + Write), root: &Path) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() == MAX_HEADERS {
            respond(stream, "431 Request Header Fields Too Large", "text/plain", b"Headers too large", false)?;
            return Ok(None);
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut request = httparse::Request::new(&mut headers);
    if !matches!(request.parse(&bytes), Ok(httparse::Status::Complete(_))) {
        respond(stream, "400 Bad Request", "text/plain", b"Bad request", false)?;
        return Ok(None);
    }
    let head = request.method == Some("HEAD");
    if request.method != Some("GET") && !head {
        respond(stream, "405 Method Not Allowed", "text/plain", b"Use GET or HEAD", head)?;
        return Ok(None);
    }
    if request.headers.iter().any(|header| header.name.eq_ignore_ascii_case("Upgrade")
        && header.value.eq_ignore_ascii_case(b"websocket")) {
        return Ok(Some(bytes));
    }
    let path = request.path.unwrap_or("").split('?').next().unwrap_or("");
    let Ok(decoded) = percent_decode_str(path).decode_utf8() else {
        respond(stream, "400 Bad Request", "text/plain", b"Invalid path", head)?;
        return Ok(None);
    };
    if !decoded.starts_with('/') || decoded.contains(['\\', ':', '\0'])
        || decoded.split('/').any(|part| part == ".." || part == ".") {
        respond(stream, "403 Forbidden", "text/plain", b"Forbidden", head)?;
        return Ok(None);
    }
    if decoded == "/config.js" {
        respond(stream, "200 OK", "text/javascript; charset=utf-8",
            b"window.BP_DEFAULT_SERVER = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws';\n", head)?;
        return Ok(None);
    }
    let relative = if decoded == "/" { "index.html" } else { decoded.trim_start_matches('/') };
    let path = match root.join(relative).canonicalize() {
        Ok(path) if path.starts_with(root) && path.is_file() => path,
        _ => {
            respond(stream, "404 Not Found", "text/plain", b"Not found", head)?;
            return Ok(None);
        }
    };
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            respond(stream, "404 Not Found", "text/plain", b"Not found", head)?;
            return Ok(None);
        }
    };
    let content_type = match path.extension().and_then(|extension| extension.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "css" => "text/css; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ttf" => "font/ttf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    };
    let length = file.metadata()?.len();
    write_headers(stream, "200 OK", content_type, length)?;
    if !head {
        io::copy(&mut file.take(length), stream)?;
    }
    Ok(None)
}

fn write_headers(stream: &mut impl Write, status: &str, content_type: &str, length: u64) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nConnection: close\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\n")?;
    if status.starts_with("405") {
        write!(stream, "Allow: GET, HEAD\r\n")?;
    }
    write!(stream, "\r\n")
}

fn respond(stream: &mut impl Write, status: &str, content_type: &str, body: &[u8], head: bool) -> io::Result<()> {
    write_headers(stream, status, content_type, body.len() as u64)?;
    if !head { stream.write_all(body)?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Exchange {
        request: io::Cursor<Vec<u8>>,
        response: Vec<u8>,
    }

    impl Read for Exchange {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.request.read(buffer)
        }
    }

    impl Write for Exchange {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.response.write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn malformed_and_oversized_http_requests_are_rejected() {
        for (request, status) in [
            ("GET / HTTP/9.9\r\n\r\n".to_string(), "400"),
            ("GET /%ff HTTP/1.1\r\n\r\n".to_string(), "400"),
            (format!("GET / HTTP/1.1\r\n{}\r\n", "X: value\r\n".repeat(65)), "400"),
            (format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEADERS)), "431"),
        ] {
            let mut exchange = Exchange { request: io::Cursor::new(request.into_bytes()), response: Vec::new() };
            assert!(route(&mut exchange, Path::new(".")).unwrap().is_none());
            assert!(exchange.response.starts_with(format!("HTTP/1.1 {status}").as_bytes()));
        }
    }

    #[test]
    fn upgrade_preserves_headers_and_leaves_following_bytes_for_websocket() {
        let headers = b"GET /ws HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
        let mut request = headers.to_vec();
        request.extend_from_slice(b"frame");
        let mut exchange = Exchange { request: io::Cursor::new(request), response: Vec::new() };
        assert_eq!(route(&mut exchange, Path::new(".")).unwrap().unwrap(), headers);
        let mut remaining = String::new();
        exchange.read_to_string(&mut remaining).unwrap();
        assert_eq!(remaining, "frame");
        assert!(exchange.response.is_empty());
    }
}