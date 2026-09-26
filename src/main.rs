use myagentrust::{AppState, create_app, db::create_pool};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")?;
    let pool = create_pool(&database_url).await?;

    sqlx::migrate!().run(&pool).await?;

    let state = AppState { db: pool };
    let app = create_app(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:9000").await?;

    axum::serve(listener, app).await?;

    Ok(())
}
