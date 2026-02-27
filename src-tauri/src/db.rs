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

pub type SqlClient = Client<Compat<TcpStream>>;

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
            return Err("Windows Authentication is not supported yet".into());
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

    let tcp = if instance.is_some() {
        TcpStream::connect_named(&tib_config)
            .await
            .map_err(|e| format!("Named instance resolution failed for '{}': {e}", config.server_name))?
    } else {
        TcpStream::connect(tib_config.get_addr())
            .await
            .map_err(|e| format!("TCP connection to '{}:{}' failed: {e}", host, port))?
    };

    tcp.set_nodelay(true)
        .map_err(|e| format!("Failed to set TCP_NODELAY: {e}"))?;

    let client = Client::connect(tib_config, tcp.compat_write())
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
