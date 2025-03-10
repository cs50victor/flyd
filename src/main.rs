use actix_web::{
    App, HttpRequest, HttpResponse, HttpServer, Responder, get, middleware, post, web,
};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct NewMachineRequest {
    app_name: String,
    #[serde(default)]
    use_private_api: bool,
    #[serde(flatten)]
    config: MachineConfig,
}

#[derive(Deserialize, Serialize)]
struct MachineConfig {
    name: Option<String>,
    region: Option<String>,
    #[serde(flatten)]
    other: serde_json::Value,
}

#[derive(Deserialize)]
struct ListMachinesRequest {
    app_name: String,
    #[serde(default)]
    use_private_api: bool,
    #[serde(default)]
    include_deleted: bool,
    region: Option<String>,
}

fn prepare_request(
    req: &HttpRequest,
    use_private: bool,
) -> Result<(reqwest::header::HeaderMap, String), HttpResponse> {
    let auth_header = match req.headers().get(actix_web::http::header::AUTHORIZATION) {
        Some(header) => header,
        None => return Err(HttpResponse::Unauthorized().body("Authorization header required")),
    };

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_bytes(auth_header.as_bytes())
            .map_err(|e| HttpResponse::InternalServerError().body(e.to_string()))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let api_hostname = if use_private {
        "http://fly-api.internal:4280"
    } else {
        "https://api.machines.dev"
    };

    Ok((headers, api_hostname.to_string()))
}

#[post("/v0/machines/new")]
async fn create_machine(
    req: HttpRequest,
    body: web::Json<NewMachineRequest>,
    http_client: web::Data<reqwest::Client>,
) -> impl Responder {
    let (headers, api_hostname) = match prepare_request(&req, body.use_private_api) {
        Ok(result) => result,
        Err(response) => return response,
    };

    let config = serde_json::to_value(&body.config).unwrap_or_default();

    let url = format!("{}/v1/apps/{}/machines", api_hostname, body.app_name);

    let response = match http_client
        .post(&url)
        .headers(headers)
        .json(&config)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => {
            return HttpResponse::InternalServerError().body(format!("API request failed: {}", e));
        }
    };

    let json = match response.json::<serde_json::Value>().await {
        Ok(json) => json,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .body(format!("Failed to read response body: {}", e));
        }
    };

    HttpResponse::Ok().json(json)
}

#[get("/v0/machines/list")]
async fn list_machines(
    req: HttpRequest,
    query: web::Query<ListMachinesRequest>,
    http_client: web::Data<reqwest::Client>,
) -> impl Responder {
    let (headers, api_hostname) = match prepare_request(&req, query.use_private_api) {
        Ok(result) => result,
        Err(response) => return response,
    };

    let mut url = match reqwest::Url::parse(&format!(
        "{}/v1/apps/{}/machines",
        api_hostname, query.app_name
    )) {
        Ok(url) => url,
        Err(e) => {
            return HttpResponse::InternalServerError().body(format!("Failed to parse URL: {}", e));
        }
    };

    {
        let mut query_pairs = url.query_pairs_mut();

        if query.include_deleted {
            query_pairs.append_pair("include_deleted", "true");
        }
        if let Some(region) = &query.region {
            query_pairs.append_pair("region", region);
        }
    }

    let response = match http_client.get(url).headers(headers).send().await {
        Ok(response) => response,
        Err(e) => {
            return HttpResponse::InternalServerError().body(format!("API request failed: {}", e));
        }
    };

    let machines = match response.json::<serde_json::Value>().await {
        Ok(machines) => machines,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .body(format!("Failed to read response body: {}", e));
        }
    };
    HttpResponse::Ok().json(machines)
}

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("flyd!")
}

#[get("/health")]
async fn health_check() -> impl Responder {
    HttpResponse::Ok().body("YES!")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    #[cfg(debug_assertions)]
    dotenvy::from_filename_override(".env.local").ok();

    pretty_env_logger::formatted_builder()
        .filter_module("flyd", log::LevelFilter::Info)
        .filter_module("actix", log::LevelFilter::Info)
        .init();

    let reqwest_client = reqwest::Client::default();

    log::info!("flyd");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(reqwest_client.clone()))
            .wrap(middleware::Logger::new("IP - %a | Time - %D ms"))
            .service(hello)
            .service(create_machine)
            .service(list_machines)
            .service(health_check)
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
