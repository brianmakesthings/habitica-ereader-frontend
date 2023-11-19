use askama::Template;
use axum::{
    extract::Path,
    response::{Html, IntoResponse},
    routing, Router,
};
use chrono::{prelude::*, Duration};
use dotenv::dotenv;
use error_chain::error_chain;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use tower_http::services::ServeDir;

error_chain! {
    foreign_links {
        Io(std::io::Error);
        HttpRequest(reqwest::Error);
    }
}

#[derive(Deserialize)]
struct TaskResponse {
    success: bool,
    data: Vec<Task>,
    #[serde(skip)]
    notifications: String,
}

#[derive(Deserialize, Debug)]
struct Task {
    _id: String,
    #[serde(rename = "userId")]
    user_id: String,
    text: String,
    #[serde(rename = "type")]
    task_type: String,
    notes: String,
    value: f32,
    priority: f32,
    attribute: String,
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
    let current_date_with_offset = chrono::Local::today() - Duration::hours(day_start_hour.into());
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

async fn root() -> impl IntoResponse {
    let env_variables = env::vars().collect::<HashMap<_, _>>();
    let api_key = env_variables["HABITICA_API_KEY"].clone();
    let user_id = env_variables["HABITICA_USER_ID"].clone();
    let day_start_hour = get_day_start_hour(&api_key, &user_id).await;
    let tasks = get_due_tasks(&api_key, &user_id, day_start_hour)
        .await
        .unwrap();
    let index_html = IndexTemplate { tasks }.render().unwrap();
    (StatusCode::OK, Html(index_html).into_response())
}

async fn clicked(Path(task_id): Path<String>) {
    println!("clicked {task_id}!");
    let env_variables = env::vars().collect::<HashMap<_, _>>();
    let api_key = env_variables["HABITICA_API_KEY"].clone();
    let user_id = env_variables["HABITICA_USER_ID"].clone();

    score_task(&api_key, &user_id, &task_id, "up")
        .await
        .unwrap();
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    // let env_map: HashMap<_, _> = env::vars().collect();
    // let api_key = env_map["HABITICA_API_KEY"].clone();

    // let serve_dir = ServeDir::new("assets").
    let router = Router::new()
        .route("/", routing::get(root))
        // .route("/force_refresh", routing::get(force_refresh))
        .nest_service("/static", ServeDir::new("./static"))
        .route("/complete/:id", routing::post(clicked));

    let port = 3002;
    let address = SocketAddr::from(([0, 0, 0, 0], port));

    println!("Server started on {address}");

    axum::Server::bind(&address)
        .serve(router.into_make_service())
        .await
        .unwrap();
    Ok(())
}
