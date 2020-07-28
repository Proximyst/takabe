use crate::prelude::*;
use actix_web::web::{self, Data, Query};
use actix_web::{get, HttpRequest, HttpResponse, Responder};
use rand::distributions::{Distribution, Uniform};
use rand::Rng;
use serde::Deserialize;
use std::collections::HashMap;
use tera::Tera;

pub struct TokenData(pub String);

#[derive(Debug, Deserialize)]
pub struct CreateOpts {
    token: String,
    url: String,

    #[serde(default)]
    path: Option<String>,
}

#[get("/new")]
pub async fn create(
    query: Query<HashMap<String, String>>,
    request: HttpRequest,
    tera: Data<Tera>,
    token: Data<TokenData>,
    db: Data<SqlitePool>,
) -> impl Responder {
    let qtoken = query.get("token");
    let qpath = query.get("shortened");
    let qurl = match query.get("url") {
        Some(u) => u,
        None => {
            return HttpResponse::BadRequest().content_type("text/html").body(
                tera.render("index.html", &tera::Context::new())
                    .expect("invalid tera template: index.html"),
            )
        }
    };

    if qtoken != Some(&token.0) {
        return HttpResponse::Unauthorized().content_type("text/html").body(
            tera.render("unauthorised.html", &tera::Context::new())
                .expect("invalid tera template: unauthorised.html"),
        );
    }

    let path = match qpath {
        Some(p) if !p.is_empty() => p.to_lowercase(),
        _ => rand::thread_rng()
            .sample_iter(TokenDistribution)
            .take(6)
            .collect(),
    };

    sqlx::query("INSERT INTO redirects (token, url) VALUES (?, ?)")
        .bind(&path)
        .bind(qurl)
        .execute(db.as_ref())
        .await
        .expect("sqlite insert unsuccessful");

    HttpResponse::Ok().content_type("text/html").body(
        tera.render(
            "success.html",
            &build_ctx(move |c| {
                c.insert(
                    "url",
                    &format!(
                        "{}/{}",
                        request
                            .headers()
                            .get("Host")
                            .map(|v| v.to_str().ok())
                            .flatten()
                            .map(str::to_owned)
                            .unwrap_or_else(
                                || std::env::var("BASE_URL").expect("no BASE_URL specified")
                            ),
                        path,
                    ),
                );
            }),
        )
        .expect("invalid tera template: index.html"),
    )
}

#[get("/")]
pub async fn index(tera: Data<Tera>) -> impl Responder {
    HttpResponse::Ok().content_type("text/html").body(
        tera.render("index.html", &tera::Context::new())
            .expect("invalid tera template: index.html"),
    )
}

#[get("/{redir}")]
pub async fn redirect(
    path: web::Path<(String,)>,
    db: Data<SqlitePool>,
    tera: Data<Tera>,
) -> impl Responder {
    let url: (String,) = match sqlx::query_as("SELECT url FROM redirects WHERE token = ?")
        .bind(&path.0)
        .fetch_one(db.as_ref())
        .await
    {
        Ok(res) => res,
        Err(e) => {
            warn!("Could not find redirect {}: {:?}", path.0, e);
            return HttpResponse::NotFound().content_type("text/html").body(
                tera.render("not-found.html", &tera::Context::new())
                    .expect("invalid tera template: not-found.html"),
            );
        }
    };

    HttpResponse::PermanentRedirect()
        .header("Location", url.0.as_str())
        .finish()
}

#[derive(Debug)]
struct TokenDistribution;

impl Distribution<char> for TokenDistribution {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> char {
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        const RANGE: usize = CHARSET.len();
        let uniform = Uniform::from(0..RANGE);
        CHARSET[uniform.sample(rng)] as char
    }
}

fn build_ctx(block: impl FnOnce(&mut tera::Context)) -> tera::Context {
    let mut ctx = tera::Context::new();
    block(&mut ctx);
    ctx
}
