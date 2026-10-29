//! Disposable HTTP/HTTPS/proxy fixture. No Python/curl and no product runtime
//! hooks are needed to test streaming, trust, redirects and target networking.
use anyhow::{bail, Context, Result};
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    ServerConfig,
};
use std::{io::Write, net::TcpListener, path::PathBuf, sync::Arc};
#[path = "../tests/support/http.rs"]
mod http;
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let address = args
        .next()
        .context("Expected address and fixture directory")?;
    let root = PathBuf::from(args.next().context("Expected fixture directory")?);
    let tls = match args.next().as_deref() {
        None => None,
        Some("--tls") => {
            let certificates = CertificateDer::pem_file_iter(root.join("server.pem"))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let key = PrivateKeyDer::from_pem_file(root.join("server-key.pem"))?;
            Some(Arc::new(
                ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(certificates, key)?,
            ))
        }
        _ => bail!("Unexpected fixture argument"),
    };
    let listener = TcpListener::bind(&address)?;
    println!("{}", listener.local_addr()?);
    std::io::stdout().flush()?;
    for connection in listener.incoming() {
        let connection = connection?;
        let root = root.clone();
        let tls = tls.clone();
        std::thread::spawn(move || {
            let _ = http::handle(connection, &root, tls);
        });
    }
    Ok(())
}
