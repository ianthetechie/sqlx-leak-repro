#[macro_use]
extern crate log;

#[macro_use]
extern crate serde;

use serde_json;
use serde_urlencoded;

use failure::Error;

use sqlx::{PgPool, Row};

use hyper::http::StatusCode;
use hyper::service::{make_service_fn, service_fn};
use hyper::{header, Body, Method, Request, Response, Server};

use std::convert::TryFrom;
use std::env;
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Deserialize)]
pub struct AddRequest {
    x: f64,
    y: f64,
}

impl TryFrom<Request<Body>> for AddRequest {
    type Error = Error;

    fn try_from(value: Request<Body>) -> Result<Self, Self::Error> {
        let query = value.uri().query().unwrap_or("");
        serde_urlencoded::from_str(query).map_err(|e| e.into())
    }
}

impl AddRequest {
    pub async fn render_response(&self, pool: &PgPool) -> Result<Response<Body>, Error> {
        let mut conn = pool.acquire().await?;

        // Simulate a query that takes a bit longer than 1ms to run; this makes the test
        // far more realistic. If we just do simple addition that takes microseconds of work,
        // the race condition never seems to occur.
        let _wait = sqlx::query("SELECT pg_sleep(0.5)")
            .fetch_one(&mut conn)
            .await?;

        let query = sqlx::query("SELECT $1 + $2");

        let sum: f64 = query
            .bind(self.x)
            .bind(self.y)
            .fetch_one(&mut conn)
            .await?
            .get(0);

        let info = serde_json::json!({
            "result": sum,
        });

        let json = serde_json::to_string(&info)?;

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .expect("Failed to construct response body"))
    }
}

async fn route(req: Request<Body>, pool: PgPool) -> Result<Response<Body>, Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/health") => Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .expect("Failed to construct response body")),
        (&Method::GET, "/add") => AddRequest::try_from(req)?.render_response(&pool).await,
        _ => {
            // Return 404 not found response.
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("Failed to construct response body"))
        }
    }
}

async fn shutdown_signal() {
    // Wait for the CTRL+C signal
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    simple_logger::init_with_level(log::Level::Info).expect("Unable to initialize global logger");

    let db_pool = PgPool::builder()
        .connect_timeout(Duration::from_secs(5))
//        .test_on_acquire(false)
        .build(&env::var("DATABASE_URL").expect("Environment var DATABASE_URL is not set"))
        .await?;

    let new_service = make_service_fn(move |_| {
        // Move a clone of env data
        let db_pool = db_pool.clone();
        async move {
            Ok::<_, Error>(service_fn(move |req| {
                // Clone again to ensure that env data outlives this closure.
                route(req, db_pool.to_owned())
            }))
        }
    });

    let sock_addr = SocketAddr::from(([127, 0, 0, 1], 8000));
    let server = Server::bind(&sock_addr)
        .serve(new_service)
        .with_graceful_shutdown(shutdown_signal());

    info!("Listening on http://{}", sock_addr);

    if let Err(e) = server.await {
        error!("server error: {}", e);
    }

    Ok(())
}
