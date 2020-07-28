mod http;
mod migrations;

mod prelude {
    pub use log::{debug, error, info, trace, warn};
    pub use sqlx::prelude::*;
    pub use sqlx::SqlitePool;
    pub use std::sync::Arc;
}

use self::prelude::*;
use actix_web::{App, HttpServer};
use anyhow::{Context as _, Result};
use sqlx::sqlite::SqliteConnectOptions;
use std::env;
use std::path::PathBuf;
use strum::IntoEnumIterator as _;

#[actix_rt::main]
async fn main() {
    eprintln!(concat!(
        env!("CARGO_PKG_NAME"),
        " (v",
        env!("CARGO_PKG_VERSION"),
        ")"
    ));
    eprintln!(
        r#"
{name} Copyright (C) 2020 {}
This program comes with ABSOLUTELY NO WARRANTY.
This is free software, and you are welcome to redistribute it
under certain conditions. See the GitHub repository for more
information: <https://github.com/Proximyst/{name}>
"#,
        env!("CARGO_PKG_AUTHORS"),
        name = env!("CARGO_PKG_NAME"),
    );
    match result_main().await {
        Ok(()) => return,
        Err(e) => {
            error!("Error on running the application:\n{:?}", e);
            std::process::exit(1);
        }
    }
}

async fn result_main() -> Result<()> {
    match dotenv::dotenv() {
        Ok(_) => (),
        Err(e) if e.not_found() => (),
        Err(e) => return Err(e.into()),
    }
    pretty_env_logger::try_init()?;
    debug!("Set up env logger.");

    let token = env::var("CREATE_TOKEN")
        .context("`CREATE_TOKEN` must exist with a token for creating redirects")?;

    let tera = tera::Tera::new("templates/**/*")?;

    let db = env::var("DATABASE_FILE")
        .unwrap_or_else(|_| concat!("./", env!("CARGO_PKG_NAME"), ".db").into())
        .parse::<PathBuf>()
        .context("`DATABASE_FILE` must be a valid path")?;
    info!("Using SQLite database at `{:?}`...", db);
    trace!("DB file: {:?}", db);
    trace!(
        "Will use: `sqlite:{}`",
        db.to_str().expect("database url not valid")
    );
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(db)
            .create_if_missing(true),
    )
    .await?;
    trace!("Connection established.");

    // {{{ Database migrations
    sqlx::query(
        r#"
CREATE TABLE IF NOT EXISTS meta_version
(
    version INT NOT NULL
)
    "#,
    )
    .execute(&pool)
    .await?;
    trace!("Created meta_version");

    // I don't particularly care if this one fails.
    let _ = sqlx::query("INSERT INTO meta_version (rowid, version) VALUES (0, 0)")
        .execute(&pool)
        .await;
    trace!("Inserted version if necessary.");

    let version: (i64,) = sqlx::query_as("SELECT version FROM meta_version")
        .fetch_one(&pool)
        .await?;
    trace!("DB version fetched: {:?}", version);
    let version = version.0;

    debug!("Found DB version: {}", version);

    for migration in self::migrations::Migrations::iter().filter(|mig| (*mig as i64) > version) {
        trace!("Migration application: {}", migration as i64);
        let ver = migration as i64;
        debug!("Applying migration to V{}", ver);
        for query in migration.queries() {
            sqlx::query(&query).execute(&pool).await?;
        }
        debug!("Finished migrating to V{}, now setting version...", ver);
        sqlx::query("UPDATE meta_version SET version = ?")
            .bind(ver)
            .execute(&pool)
            .await?;
        debug!("Version set to {}", ver);
        trace!("Migration application finished: {}", ver);
    }
    // }}}

    info!("Database ready!");

    let pool2 = pool.clone();
    HttpServer::new(move || {
        App::new()
            .data(tera.clone())
            .data(http::TokenData(token.clone()))
            .data(pool2.clone())
            .service(http::create)
            .service(http::index)
            .service(http::redirect)
    })
    .bind(env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8080".into()))?
    .run()
    .await?;

    pool.close().await;

    Ok(())
}
