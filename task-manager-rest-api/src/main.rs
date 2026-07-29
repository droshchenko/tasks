use std::sync::Arc;

use app::AppContext;

mod app;
mod auth;
mod board;
mod http_server;
mod mappers;
mod mcp;
mod postgres;
mod scripts;
mod settings;
mod subscribers;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() {
    let settings_reader = settings::SettingsReader::new(".task-manager-rest-api").await;
    let settings_reader = Arc::new(settings_reader);

    let mut service_context = service_sdk::ServiceContext::new(settings_reader.clone()).await;

    let app = Arc::new(AppContext::new(settings_reader).await);

    // Read the whole product into memory BEFORE the HTTP server starts serving. Every read path assumes
    // the board is loaded; doing this in the background would serve an empty board for the first few
    // hundred milliseconds after every deploy, which looks exactly like data loss.
    scripts::load_state(&app).await;

    // Three surfaces on one HTTP server, in one process, over one copy of the state:
    //   /api/*  — reads for the UI and the configuration CRUD
    //   /mcp    — every task mutation
    //   /ws     — the invalidation push that tells Home to re-read
    //
    // That co-location is what makes the WebSocket fan-out a function call rather than a message bus,
    // and it is why this service runs as a single instance.
    let mcp_middleware = Arc::new(mcp::build_middleware(app.clone()));

    service_context.configure_http_server(move |builder| {
        let ws_middleware = Arc::new(
            service_sdk::my_http_server::web_sockets::MyWebsocketMiddleware::new(
                "/ws",
                Arc::new(http_server::ws::HomeWsCallbacks::new(app.clone())),
                service_sdk::my_logger::LOGGER.clone(),
            ),
        );

        builder.register_custom_middleware(ws_middleware);
        builder.register_custom_middleware(mcp_middleware.clone());

        http_server::build_controllers(&app, builder);
    });

    service_context.start_application().await;
}
