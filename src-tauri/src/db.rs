use serde::Deserialize;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel, SqlBrowser};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionConfig {
    pub server_name: String,
    pub authentication: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub encrypt: String,
    pub trust_cert: bool,
}

use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub enum TransportStream {
    Tcp(TcpStream),
    #[cfg(windows)]
    NamedPipe(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl AsyncRead for TransportStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(windows)]
            TransportStream::NamedPipe(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TransportStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(windows)]
            TransportStream::NamedPipe(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_flush(cx),
            #[cfg(windows)]
            TransportStream::NamedPipe(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(windows)]
            TransportStream::NamedPipe(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub type SqlClient = Client<Compat<TransportStream>>;

pub async fn connect(config: &ConnectionConfig) -> Result<SqlClient, String> {
    let mut tib_config = Config::new();

    let (host, port, instance) = parse_server_name(&config.server_name)?;
    tib_config.host(&host);
    tib_config.port(port);
    if let Some(inst) = &instance {
        tib_config.instance_name(inst);
    }

    if !config.database.is_empty() {
        tib_config.database(&config.database);
    }

    match config.authentication.as_str() {
        "windows" => {
            #[cfg(windows)]
            tib_config.authentication(AuthMethod::Integrated);
            #[cfg(not(windows))]
            return Err("Windows Authentication is only supported on Windows".into());
        }
        _ => {
            tib_config.authentication(AuthMethod::sql_server(&config.username, &config.password));
        }
    }

    let encryption = match config.encrypt.as_str() {
        "optional" => EncryptionLevel::Off,
        "strict" => EncryptionLevel::Required,
        _ => EncryptionLevel::Required,
    };
    tib_config.encryption(encryption);

    if config.trust_cert {
        tib_config.trust_cert();
    }

    tib_config.application_name("SimpleSQLProfiler");

    let tcp_result = if instance.is_some() {
        TcpStream::connect_named(&tib_config).await.map_err(|e| e.to_string())
    } else {
        TcpStream::connect(tib_config.get_addr()).await.map_err(|e| e.to_string())
    };

    let stream = match tcp_result {
        Ok(tcp) => {
            tcp.set_nodelay(true)
                .map_err(|e| format!("Failed to set TCP_NODELAY: {e}"))?;
            TransportStream::Tcp(tcp)
        }
        Err(tcp_err) => {
            #[cfg(windows)]
            {
                if host.eq_ignore_ascii_case("localhost") || host == "." || host == "127.0.0.1" {
                    let pipe_name = match &instance {
                        Some(inst) => format!(r"\\.\pipe\MSSQL${}\sql\query", inst),
                        None => r"\\.\pipe\sql\query".to_string(),
                    };
                    
                    match tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe_name) {
                        Ok(pipe) => TransportStream::NamedPipe(pipe),
                        Err(pipe_err) => return Err(format!("TCP connection failed ({}) and Named Pipe fallback failed ({})", tcp_err, pipe_err)),
                    }
                } else {
                    return Err(format!("TCP connection to '{}:{}' failed: {}", host, port, tcp_err));
                }
            }
            #[cfg(not(windows))]
            return Err(format!("TCP connection to '{}:{}' failed: {}", host, port, tcp_err));
        }
    };

    let client = Client::connect(tib_config, stream.compat_write())
        .await
        .map_err(|e| format!("SQL Server connection failed: {e}"))?;

    Ok(client)
}

fn parse_server_name(server_name: &str) -> Result<(String, u16, Option<String>), String> {
    let (addr, explicit_port) = if let Some(comma_idx) = server_name.rfind(',') {
        let port_str = server_name[comma_idx + 1..].trim();
        let port: u16 = port_str
            .parse()
            .map_err(|_| format!("Invalid port: {port_str}"))?;
        (&server_name[..comma_idx], Some(port))
    } else {
        (server_name, None)
    };

    let (host, instance) = if let Some(slash_idx) = addr.find('\\') {
        (&addr[..slash_idx], Some(addr[slash_idx + 1..].to_string()))
    } else {
        (addr, None)
    };

    let port = explicit_port.unwrap_or(if instance.is_some() { 1434 } else { 1433 });

    Ok((host.to_string(), port, instance))
}
