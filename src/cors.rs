use http_body_util::{BodyExt, Empty};
use httpserver::{Bytes, HttpContext, HttpErr, HttpResponse, Next};
use hyper::{
    header::{
        ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
        ALLOW,
    },
    http::HeaderValue,
    Method,
};

pub struct CorsMiddleware;

#[async_trait::async_trait]
impl httpserver::HttpMiddleware for CorsMiddleware {
    async fn handle<'a>(&'a self, ctx: HttpContext, next: Next<'a>) -> HttpResponse {
        let rid = ctx.id;
        let is_options = ctx.req.method() == Method::OPTIONS;
        let allow_host = HeaderValue::from_str("*").unwrap();
        if is_options {
            hyper::Response::builder()
                .header(ACCESS_CONTROL_ALLOW_HEADERS, allow_host.clone())
                .header(ACCESS_CONTROL_ALLOW_METHODS, allow_host.clone())
                .header(ACCESS_CONTROL_ALLOW_ORIGIN, allow_host.clone())
                .header(ALLOW, HeaderValue::from_str("GET,HEAD,OPTIONS").unwrap())
                .body(Empty::<Bytes>::new().boxed())
                .http_err(rid)
        } else {
            let mut res = next.run(ctx).await?;
            let h = res.headers_mut();
            h.append(ACCESS_CONTROL_ALLOW_HEADERS, allow_host.clone());
            h.append(ACCESS_CONTROL_ALLOW_METHODS, allow_host.clone());
            h.append(ACCESS_CONTROL_ALLOW_ORIGIN, allow_host.clone());
            Ok(res)
        }
    }
}
