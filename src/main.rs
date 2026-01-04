mod config;
mod hosts;
mod ipmi;
mod pxe;

use axum::Router;
use axum::routing::{get, put};

use crate::config::Config;
use crate::hosts::{ipmi_host_put_handler, ipmi_hosts_handler};
use crate::pxe::{pxe_boot_handler, pxe_file_handler};

#[derive(rust_embed::RustEmbed, Clone)]
#[folder = "web/dist"]
struct Assets;

use tower_http::cors::{self, CorsLayer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = std::fs::read("config.toml")?;
    let mut config: Config = toml::from_slice(&config)?;

    match (&config.ipmi.password, &config.ipmi.password_file) {
        (Some(_), None) => (),
        (None, Some(path)) => {
            let password = std::fs::read_to_string(path)?;
            config.ipmi.password = password.trim_end_matches('\n').to_owned().into();
        }
        (None, None) => anyhow::bail!("Either `password` or `password_file` must be provided"),
        (Some(_), Some(_)) => anyhow::bail!("Cannot set both `password` and `password_file`"),
    }

    let cors = CorsLayer::new()
        .allow_origin(cors::Any);

    let serve_assets = axum_embed::ServeEmbed::<Assets>::new();
    let app = Router::new()
        .route("/pxe/v1/boot/{mac}", get(pxe_boot_handler))
        .route("/pxe/file/{hash}/{*path}", get(pxe_file_handler))
        .route("/hosts", get(ipmi_hosts_handler))
        .route("/host/{hostname}", put(ipmi_host_put_handler))
        .fallback_service(serve_assets)
        .layer(cors)
        .with_state(config);

    let port: u16 = std::env::var("PORT")
        .map(|p| p.parse().expect("Port is invalid"))
        .unwrap_or(8080);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await?;
    Ok(())
}
