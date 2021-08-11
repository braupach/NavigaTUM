#[macro_use]
extern crate lazy_static;
use serde_json;
use std::fs;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::client::{Client, ClientBuilder, Connector};
use actix_web::{get, http, middleware, web, App, HttpRequest, HttpResponse, HttpServer, Result};

mod search;

lazy_static! {
    static ref JSON_DATA: serde_json::map::Map<String, serde_json::Value> = {
        let data = fs::read_to_string("data/api_data.json").expect("Cannot open data file");
        serde_json::from_str(&data).expect("Could not parse JSON file")
    };
}

#[get("/get/{id}")]
async fn get_handler(web::Path(id): web::Path<String>) -> Result<HttpResponse> {
    if JSON_DATA.contains_key(&id) {
        Ok(HttpResponse::Ok().json(JSON_DATA.get(&id).unwrap()))
    } else {
        Ok(HttpResponse::NotFound().body("Not found".to_string()))
    }
}

#[get("/search/{q}")]
async fn search_handler(
    _req: HttpRequest,
    web::Path(q): web::Path<String>,
    client: web::Data<Client>,
) -> Result<HttpResponse> {
    let search_results = search::do_search(q, client).await?;
    let result_json = serde_json::to_string(&search_results)?;

    Ok(HttpResponse::Ok()
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(result_json))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    JSON_DATA.contains_key("");

    HttpServer::new(|| {
        let cors = Cors::default().allow_any_origin();

        App::new()
            .wrap(cors)
            .wrap(middleware::Compress::default())
            .data(
                ClientBuilder::new()
                    .connector(
                        Connector::new()
                            .conn_keep_alive(Duration::new(30, 0))
                            .finish(),
                    )
                    .finish(),
            )
            .service(get_handler)
            .service(search_handler)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
