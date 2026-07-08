use async_trait::async_trait;
use pingora::http::ResponseHeader;
use pingora::prelude::*;

pub struct QueriaProxy {
    api_upstream: String,
    mcp_upstream: String,
    admin_upstream: String,
}

#[async_trait]
impl ProxyHttp for QueriaProxy {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

    async fn request_filter(&self, session: &mut Session, _ctx: &mut Self::CTX) -> Result<bool> {
        let path = session.req_header().uri.path();
        if path == "/healthz" || path == "/health" {
            let mut header = ResponseHeader::build(200, Some(2)).unwrap();
            header.insert_header("content-type", "text/plain").unwrap();
            header.insert_header("connection", "close").unwrap();
            session
                .write_response_header(Box::new(header), true)
                .await?;
            session.write_response_body(Some("OK".into()), true).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let path = session.req_header().uri.path();
        let target = if path.starts_with("/api/") {
            &self.api_upstream
        } else if path.starts_with("/mcp") {
            &self.mcp_upstream
        } else {
            &self.admin_upstream
        };

        tracing::debug!(path = %path, target = %target, "routing request");

        let peer = Box::new(HttpPeer::new(target.as_str(), false, "".to_string()));
        Ok(peer)
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()> {
        // Forward client IP
        if let Some(client_addr) = session.client_addr() {
            let client_ip = client_addr
                .as_inet()
                .map(|inet| inet.ip().to_string())
                .unwrap_or_else(|| client_addr.to_string());
            upstream_request
                .insert_header("X-Forwarded-For", &client_ip)
                .unwrap();
            upstream_request
                .insert_header("X-Real-IP", &client_ip)
                .unwrap();
        }

        // Forward Host header
        if let Some(host) = session.get_header("Host") {
            upstream_request.insert_header("Host", host).unwrap();
        }

        // Forward request ID
        let req_id = session
            .get_header("X-Request-ID")
            .map(|val| val.to_str().unwrap_or("").to_owned())
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        upstream_request
            .insert_header("X-Request-ID", &req_id)
            .unwrap();

        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let config = queria_core::AppConfig::from_env()
        .unwrap_or_else(|_| queria_core::AppConfig::default_local());

    queria_observability::init_json_tracing("queria-proxy", &config.log_level);

    let api_upstream =
        std::env::var("QUERIA_API_UPSTREAM").unwrap_or_else(|_| "127.0.0.1:17671".to_string());
    let mcp_upstream =
        std::env::var("QUERIA_MCP_UPSTREAM").unwrap_or_else(|_| "127.0.0.1:17672".to_string());
    let admin_upstream =
        std::env::var("QUERIA_ADMIN_UPSTREAM").unwrap_or_else(|_| "127.0.0.1:4321".to_string());

    tracing::info!(
        proxy_addr = %config.proxy_addr,
        api_upstream = %api_upstream,
        mcp_upstream = %mcp_upstream,
        admin_upstream = %admin_upstream,
        "initializing Pingora proxy"
    );

    let mut server =
        Server::new(None).map_err(|e| anyhow::anyhow!("failed to create Pingora server: {e:?}"))?;
    server.bootstrap();

    let proxy = QueriaProxy {
        api_upstream,
        mcp_upstream,
        admin_upstream,
    };

    let mut service = http_proxy_service(&server.configuration, proxy);
    service.add_tcp(&config.proxy_addr);

    server.add_service(service);
    server.run_forever();
}
