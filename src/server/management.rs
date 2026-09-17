//! Browser management has its own origin and a per-server bearer credential.
use super::*;

#[derive(Clone)]
pub(super) struct Management {
    apps: Inventory,
    origin: String,
    authorization: String,
    gateway_port: u16,
}

pub(super) async fn bind(
    apps: Inventory,
    gateway_port: u16,
) -> Result<(tokio::net::TcpListener, Router, String), String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("cannot bind management dashboard: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| "cannot generate management token")?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let origin = format!("http://127.0.0.1:{port}");
    let url = format!("{origin}/#{token}");
    let state = Management {
        apps,
        origin,
        authorization: format!("Bearer {token}"),
        gateway_port,
    };
    Ok((
        listener,
        Router::new().fallback(dispatch).with_state(state),
        url,
    ))
}

async fn dispatch(State(state): State<Management>, request: Request) -> Response {
    let mut response = handle(&state, request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

async fn handle(state: &Management, request: Request) -> Response {
    let headers = request.headers();
    if headers.get(header::HOST).and_then(|h| h.to_str().ok())
        != state.origin.strip_prefix("http://")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Reject cross-origin browser requests, even with a copied credential. No CORS.
    if headers
        .get(header::ORIGIN)
        .is_some_and(|h| h != state.origin.as_str())
        || headers
            .get("sec-fetch-site")
            .is_some_and(|h| h != "same-origin" && h != "none")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = request.uri().path();
    if request.method() == Method::GET {
        match path {
            "/" => return Html(include_str!("../../runtime/management.html")).into_response(),
            "/dashboard.js" => {
                return (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../runtime/management.js"),
                )
                    .into_response();
            }
            _ => {}
        }
    }
    if path != "/api/apps" {
        return StatusCode::NOT_FOUND.into_response();
    }
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        != Some(state.authorization.as_str())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let command = if request.method() == Method::GET {
        control::Command {
            action: control::Action::Status,
            app: None,
        }
    } else if request.method() == Method::POST {
        if headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) != Some(&state.origin) {
            return StatusCode::FORBIDDEN.into_response();
        }
        if headers
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            != Some("application/json")
        {
            return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
        }
        let body =
            match tokio::time::timeout(Duration::from_secs(2), to_bytes(request.into_body(), 4096))
                .await
            {
                Ok(Ok(body)) => body,
                Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
                Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
            };
        match serde_json::from_slice::<control::Command>(&body) {
            Ok(command) => command,
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid lifecycle command").into_response();
            }
        }
    } else {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, POST")],
        )
            .into_response();
    };
    match manage(&state.apps, command) {
        Ok(apps) => (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"apps": apps, "gateway": format!("http://127.0.0.1:{}", state.gateway_port)}).to_string(),
        ).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
    }
}
