use askama::Template;
use axum::{
    extract::{Path, State},
    http::Request,
    middleware::Next,
    response::{Html, IntoResponse, Redirect},
    routing, Form, Router,
};
use axum_extra::extract::{cookie::Cookie, CookieJar};
use chrono::{prelude::*, Duration};
use dotenv::dotenv;
use error_chain::error_chain;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use tower_http::services::{ServeDir, ServeFile};

error_chain! {
    foreign_links {
        Io(std::io::Error);
        HttpRequest(reqwest::Error);
    }
}

#[derive(Deserialize)]
struct TaskResponse {
    data: Vec<Task>,
}

#[derive(Deserialize, Debug)]
struct Task {
    _id: String,
    text: String,
    repeat: Option<RepeatSchedule>,
    completed: bool,
}

#[derive(Deserialize, Debug, Copy, Clone)]
struct RepeatSchedule {
    su: bool,
    m: bool,
    t: bool,
    w: bool,
    th: bool,
    f: bool,
    s: bool,
}

#[derive(Deserialize)]
struct UserResponse {
    data: User,
}

#[derive(Deserialize)]
struct User {
    preferences: Preferences,
}

#[derive(Deserialize)]
struct Preferences {
    #[serde(rename = "dayStart")]
    day_start: i32,
}

async fn get_due_tasks(api_key: &str, user_id: &str, day_start_hour: i32) -> Result<Vec<Task>> {
    let all_tasks = get_all_tasks(api_key, user_id).await?;
    Ok(all_tasks
        .into_iter()
        .filter(|task| {
            task.repeat
                .map_or(true, |schedule| task_due_today(schedule, day_start_hour))
        })
        .collect())
}

async fn get_all_tasks(api_key: &str, user_id: &str) -> Result<Vec<Task>> {
    let client = reqwest::Client::new();
    let res = client
        .get("https://habitica.com/api/v3/tasks/user")
        .header("x-client", "test-app")
        .header("x-api-user", user_id)
        .header("x-api-key", api_key)
        .send()
        .await
        .unwrap();

    Ok(res.json::<TaskResponse>().await.unwrap().data)
}

async fn score_task(
    api_key: &str,
    user_id: &str,
    task_id: &str,
    direction: &str,
) -> Result<StatusCode> {
    let client = reqwest::Client::new();
    let query_string = format!(
        "https://habitica.com/api/v3/tasks/{}/score/{}",
        task_id, direction
    );
    // let query_string = "https://habitica.com/api/v3/tasks/d434eb4e-ca94-40f5-9794-81ae805990fa/score/up";
    let res = client
        .post(query_string)
        .header("x-client", "test-app")
        .header("x-api-user", user_id)
        .header("x-api-key", api_key)
        .header("Content-Length", 0)
        .send()
        .await
        .unwrap();

    match res.status() {
        StatusCode::OK => Ok(res.status()),
        _ => panic!("Got Error code {}", res.status()),
    }
}

fn task_due_today(repeat_schedule: RepeatSchedule, day_start_hour: i32) -> bool {
    let current_date_with_offset = chrono::Local::now() - Duration::hours(day_start_hour.into());
    let day_of_week = current_date_with_offset.weekday();
    match day_of_week {
        chrono::Weekday::Sun => repeat_schedule.su,
        chrono::Weekday::Mon => repeat_schedule.m,
        chrono::Weekday::Tue => repeat_schedule.t,
        chrono::Weekday::Wed => repeat_schedule.w,
        chrono::Weekday::Thu => repeat_schedule.th,
        chrono::Weekday::Fri => repeat_schedule.f,
        chrono::Weekday::Sat => repeat_schedule.s,
    }
}

async fn get_day_start_hour(api_key: &str, user_id: &str) -> i32 {
    let client = reqwest::Client::new();
    let res = client
        .get("https://habitica.com/api/v3/user?userFields=preferences")
        .header("x-client", "test-app")
        .header("x-api-user", user_id)
        .header("x-api-key", api_key)
        .header("Content-Length", 0)
        .send()
        .await
        .unwrap();

    let data = res.json::<UserResponse>().await.unwrap().data;
    data.preferences.day_start
}

#[derive(Template)]
#[template(path = "./index.html")]
struct IndexTemplate {
    tasks: Vec<Task>,
}

async fn root(State(state): State<AppState>) -> impl IntoResponse {
    let day_start_hour = get_day_start_hour(&state.habitica_api_key, &state.habitica_user_id).await;
    let tasks = get_due_tasks(
        &state.habitica_api_key,
        &state.habitica_user_id,
        day_start_hour,
    )
    .await
    .unwrap();
    let index_html = IndexTemplate { tasks }.render().unwrap();
    (StatusCode::OK, Html(index_html).into_response())
}

async fn clicked(State(state): State<AppState>, Path(task_id): Path<String>) {
    println!("clicked {task_id}!");
    score_task(
        &state.habitica_api_key,
        &state.habitica_user_id,
        &task_id,
        "up",
    )
    .await
    .unwrap();
}

#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(login): Form<Login>,
) -> impl IntoResponse {
    let cookie = jar.get("authorization_token");
    if let Some(authz_token) = cookie {
        if authz_token.value() == state.authz_token {
            return (jar, Redirect::to("/"));
        }
    }

    let form_username = login.username;
    let form_password = login.password;
    println!("{form_username}, {form_password}");

    if form_username != state.username || form_password != state.password {
        return (jar, Redirect::to("/login"));
    }

    println!("AUTHENTICATED!");

    let built_cookie = Cookie::build("authorization_token", state.authz_token)
        .path("/")
        .secure(true)
        .http_only(true)
        .permanent()
        .finish();

    (jar.add(built_cookie), Redirect::to("/"))
}

async fn authorization<B>(
    State(state): State<AppState>,
    req: Request<B>,
    next: Next<B>,
) -> impl IntoResponse {
    let headers = req.headers();
    let jar = CookieJar::from_headers(headers);

    let cookie = jar.get("authorization_token");
    if let Some(authz_token) = cookie {
        if authz_token.value() == state.authz_token {
            println!("authorized!");
            return next.run(req).await;
        }
    }

    Redirect::to("/login").into_response()
}

#[derive(Clone, Debug)]
struct AppState {
    habitica_api_key: String,
    habitica_user_id: String,
    username: String,
    password: String,
    authz_token: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    let env_map: HashMap<_, _> = env::vars().collect();
    let state = AppState {
        habitica_api_key: env_map["HABITICA_API_KEY"].clone(),
        habitica_user_id: env_map["HABITICA_USER_ID"].clone(),
        username: env_map["USERNAME"].clone(),
        password: env_map["PASSWORD"].clone(),
        authz_token: env_map["AUTHZ_TOKEN"].clone(),
    };

    println!("{:?}", state);
    println!("Current time: {}", chrono::Local::now());

    let router = Router::new()
        .route("/", routing::get(root))
        .route("/complete/:id", routing::post(clicked))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            authorization,
        ))
        .nest_service("/static", ServeDir::new("./static"))
        .route("/api/login", routing::post(login))
        .route_service("/login", ServeFile::new("./static/login.html"))
        .with_state(state);

    let port = env_map["PORT"].parse().unwrap_or(3002);
    let address = SocketAddr::from(([0, 0, 0, 0], port));

    println!("Server started on {address}");

    axum::Server::bind(&address)
        .serve(router.into_make_service())
        .await
        .unwrap();
    Ok(())
}
