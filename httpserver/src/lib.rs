use compact_str::CompactString;
use fnv::FnvHashMap;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{body::{Buf, Incoming}, header::AsHeaderName, http::HeaderValue, server::conn::http1, service};
use hyper_util::rt::TokioIo;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    borrow::Cow, collections::HashMap, convert::Infallible, fmt::Debug, future::Future, net::{Ipv4Addr, SocketAddr}, result::Result, sync::{atomic::AtomicU32, Arc}
};
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};

pub use compact_str;
pub use hyper::body::Bytes;

/* #region 宏定义 */

/// 类似anyhow::bail宏
#[macro_export]
macro_rules! http_bail {
    ($msg:literal $(,)?) => {
        return Err($crate::HttpError::Custom(String::from($msg)))
    };
    ($err:expr $(,)?) => {
        return Err($crate::HttpError::Custom($err))
    };
    ($fmt:expr, $($arg:tt)*) => {
        return Err($crate::HttpError::Custom(format!($fmt, $($arg)*)))
    };
}

/// 类似anyhow::anyhow宏
#[macro_export]
macro_rules! http_err {
    ($msg:literal $(,)?) => {
        $crate::HttpError::Custom(String::from($msg))
    };
    ($err:expr $(,)?) => {
        $crate::HttpError::Custom($err)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::HttpError::Custom(format!($fmt, $($arg)*))
    };
}

/// Batch registration API interface
///
/// ## Example
/// ```rust
/// use anyhow::Result;
/// use httpserver::{HttpContext, Response, register_apis};
///
/// async fn ping(ctx: HttpContext) -> Result<Response> { todo!() }
/// async fn login(ctx: HttpContext) -> Result<Response> { todo!() }
///
/// let mut srv = HttpServer::new(true);
/// register_apis!(srv, "/api",
///     "/ping": apis::ping,
///     "/login": apis::login,
/// );
/// ```
#[macro_export]
macro_rules! register_apis {
    ($server:expr, $base:expr, $($path:literal : $handler:expr,)+) => {
        $(
            $server.register(&$crate::compact_str::format_compact!("{}{}",
                $base, $path), $handler);
        )*
    };
}

/// Error message response returned when struct fields is Option::None
///
/// ## Example
/// ```rust
/// struct User {
///     name: Option<String>,
///     age: Option<u8>,
/// }
///
/// let user = User { name: None, age: 48 };
///
/// httpserver::check_required!(user, name, age);
/// ```
#[macro_export]
macro_rules! check_required {
    ($val:expr, $($attr:tt),+) => {
        $(
            if $val.$attr.is_none() {
                return Err($crate::HttpError::Custom(format!("{}{}", stringify!($attr), "不能为空")))
            }
        )*
    };
}

/// Error message response returned when struct fields is Option::None
///
/// ## Example
/// ```rust
/// struct User {
///     name: Option<String>,
///     age: Option<u8>,
/// }
///
/// let user = User { name: String::from("kiven"), age: 48 };
///
/// let (name, age) = httpserver::assign_required!(user, name, age);
///
/// assert_eq!("kiven", name);
/// assert_eq!(48, age);
/// ```
#[macro_export]
macro_rules! assign_required {
    ($val:expr, $($attr:tt),+) => {
        let ($($attr,)*) = (
            $(
                match &$val.$attr {
                    Some(v) => v,
                    None => return Err($crate::HttpError::Custom(format!("{}{}", stringify!($attr), " can't be null")))
                },
            )*
        );
    };
}

/// Error message response returned when expression is true
///
/// ## Example
/// ```rust
/// use httpserver::fail_if;
///
/// let age = 30;
/// fail_if!(age >= 100, "age must be range 1..100");
/// fail_if!(age >= 100, "age is {}, not in range 1..100", age);
/// ```
#[macro_export]
macro_rules! fail_if {
    ($b:expr, $msg:literal) => {
        if $b {
            return Err($crate::HttpError::Custom(String::from($msg)))
        }
    };
    ($b:expr, $($t:tt)+) => {
        if $b {
            return Err($crate::HttpError::Custom(format!($($t)*)))
        }
    };
}

/// Error message response returned when ApiResult.is_fail() == true
///
/// ## Example
/// ```rust
/// use httpserver::fail_if_api;
///
/// let f = || {
///     let ar = ApiResult::fail("open database error");
///     fail_if_api!(ar);
///     Resp::ok_with_empty()
/// }
/// assert_eq!(f(), Resp::fail_with_api_result(ApiResult::fail("open database error")));
/// ```
#[macro_export]
macro_rules! fail_if_api {
    ($ar:expr) => {
        if $ar.is_ok() {
            $ar.data
        } else {
            log::info!("ApiResult error: code = {}, msg = {}", $ar.code, $ar.msg);
            return Err($crate::HttpError::Custom(format!($ar.msg)));
        }
    };
}

/// ternary expression
///
///  ## Example
/// ```rust
/// use httpserver::ternary_expr;
///
/// let a = ternary_expr!(true, 52, 42);
/// let b = ternary_expr!(false, 52, 42);
/// assert_eq!(52, a);
/// assert_eq!(42, b);
/// ```
#[macro_export]
macro_rules! ternary_expr {
    ($b:expr, $val1:expr, $val2:expr) => {
        if $b {
            $val1
        } else {
            $val2
        }
    };
}

#[macro_export]
macro_rules! log_trace {
    (target: $target:expr, $reqid:expr, $($arg:tt)+) => (log::trace!(target: $target, "[http-req:{}] {}", $reqid, format_args!($($arg)+)));
    ($reqid:expr, $($arg:tt)+) => (log::trace!("[http-req:{}] {}", $reqid, format_args!($($arg)+)))
}

#[macro_export]
macro_rules! log_debug {
    (target: $target:expr, $reqid:expr, $($arg:tt)+) => (log::debug!(target: $target, "[http-req:{}] {}", $reqid, format_args!($($arg)+)));
    ($reqid:expr, $($arg:tt)+) => (log::debug!("[http-req:{}] {}", $reqid, format_args!($($arg)+)))
}

#[macro_export]
macro_rules! log_info {
    (target: $target:expr, $reqid:expr, $($arg:tt)+) => (log::info!(target: $target, "[http-req:{}] {}", $reqid, format_args!($($arg)+)));
    ($reqid:expr, $($arg:tt)+) => (log::info!("[http-req:{}] {}", $reqid, format_args!($($arg)+)))
}

#[macro_export]
macro_rules! log_warn {
    (target: $target:expr, $reqid:expr, $($arg:tt)+) => (log::warn!(target: $target, "[http-req:{}] {}", $reqid, format_args!($($arg)+)));
    ($reqid:expr, $($arg:tt)+) => (log::warn!("[http-req:{}] {}", $reqid, format_args!($($arg)+)))
}

#[macro_export]
macro_rules! log_error {
    (target: $target:expr, $reqid:expr, $($arg:tt)+) => (log::error!(target: $target, "[http-req:{}] {}", $reqid, format_args!($($arg)+)));
    ($reqid:expr, $($arg:tt)+) => (log::error!("[http-req:{}] {}", $reqid, format_args!($($arg)+)))
}

/* #endregion */

/// http header "Content-Type"
pub const CONTENT_TYPE: &str = "Content-Type";
/// http header "applicatoin/json; charset=UTF-8"
pub const APPLICATION_JSON: &'static str = "applicatoin/json; charset=UTF-8";

/// Simplified declaration
pub type HttpResult<T> = Result<T, HttpError>;
pub type Request = hyper::Request<http_body_util::Full<Bytes>>;
pub type Response = hyper::Response<BoxBody<Bytes, Infallible>>;
pub type HttpResponse = HttpResult<Response>;
pub type BoxHttpHandler = Box<dyn HttpHandler>;

type HttpCtxAttrs = Option<HashMap<CompactString, Value>>;
type Router = FnvHashMap<CompactString, BoxHttpHandler>;

#[derive(Error, Debug)]
pub enum HttpError {
    #[error("请求参数格式错误")]
    BodyNotJson,
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    HyperError(#[from] hyper::Error),
    #[error("{0}")]
    Custom(String),
    #[error("内部错误: {0}")]
    Unknown(u32),
}

/// use for HttpServer.run_with_callback
#[async_trait::async_trait]
pub trait RunCallback {
    async fn handle(self) -> HttpResult<()>;
}

/// api function interface
#[async_trait::async_trait]
pub trait HttpHandler: Send + Sync + 'static {
    async fn handle(&self, ctx: HttpContext) -> HttpResponse;
}

/// middleware interface
#[async_trait::async_trait]
pub trait HttpMiddleware: Send + Sync + 'static {
    async fn handle<'a>(&'a self, ctx: HttpContext, next: Next<'a>) -> HttpResponse;
}

pub trait HttpErr<T, E> {
    fn http_err(self, rid: u32) -> HttpResult<T>;
}

/// Universal API interface returns data format
#[derive(Serialize, Deserialize, Debug)]
// #[serde(rename_all = "camelCase")]
pub struct ApiResult<T> {
    /// result code (usually, 200 represents success, and 500 represents failure)
    pub code: u32,
    /// error message, When the code is equal to 500, it indicates an error message, and when the code is equal to 200, it is None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// result data, When the code is equal to 200, it indicates the specific return object
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

/// api function param
pub struct HttpContext {
    /// http request object
    pub req: Request,
    /// http request client ip address
    pub addr: SocketAddr,
    /// http request ID (each request ID is unique)
    pub id: u32,
    /// current login user ID (parsed from token, not logged in is empty)
    pub uid: CompactString,
    /// additional attributes (user-defined)
    pub attrs: HttpCtxAttrs,
}

/// Build http response object
pub struct Resp;

/// http request process object
pub struct Next<'a> {
    pub endpoint: &'a dyn HttpHandler,
    pub next_middleware: &'a [Box<dyn HttpMiddleware>],
}

/// Log middleware
pub struct AccessLog;

/// http server
pub struct HttpServer {
    prefix: CompactString,
    router: Router,
    middlewares: Vec<Box<dyn HttpMiddleware>>,
    default_handler: BoxHttpHandler,
    error_handler: fn(u32, HttpError) -> Response,
}

struct HttpServerData {
    id: AtomicU32,
    server: HttpServer,
}

impl<T, E: Debug> HttpErr<T, E> for Result<T, E> {
    #[inline]
    fn http_err(self, rid: u32) -> HttpResult<T> {
        self.map_err(|e| {
            log_error!(rid, "内部错误, {e:?}");
            HttpError::Unknown(rid)
        })
    }
}

impl From<anyhow::Error> for HttpError {
    fn from(value: anyhow::Error) -> Self {
        HttpError::Custom(value.to_string())
    }
}

impl<T> ApiResult<T> {
    /// Generate an ApiResult that represents success using the specified data
    #[inline]
    pub fn ok(data: T) -> Self {
        Self {
            code: 200,
            message: None,
            data: Some(data),
        }
    }

    /// Generate an ApiResult that represents success using empty data
    #[inline]
    pub fn ok_with_empty() -> Self {
        Self {
            code: 200,
            message: None,
            data: None,
        }
    }

    /// Generate an ApiResult indicating failure using the specified error message
    #[inline]
    pub fn fail(msg: String) -> Self {
        Self {
            code: 500,
            message: Some(msg),
            data: None,
        }
    }

    /// Generate an ApiResult representing a failure using the specified error code and error message
    #[inline]
    pub fn fail_with_code(code: u32, msg: String) -> Self {
        Self {
            code,
            message: Some(msg),
            data: None,
        }
    }

    /// Determine if ApiResult indicates successful return
    #[inline]
    pub fn is_ok(&self) -> bool {
        self.code == 200
    }

    /// Determine if ApiResult indicates a return failure
    #[inline]
    pub fn is_fail(&self) -> bool {
        self.code != 200
    }
}

impl HttpContext {
    /// Asynchronous parsing of the body content of HTTP requests in JSON format
    ///
    /// Returns:
    ///
    /// **Ok(val)**: body parse success
    ///
    /// **Err(e)**: parse error
    ///
    ///  ## Example
    /// ```rust
    /// use httpserver::{HttpContext, Response, Resp};
    ///
    /// #[derive(serde::Deserialize)]
    /// struct ReqParam {
    ///     user: Option<String>,
    ///     pass: Option<String>,
    /// }
    ///
    /// async fn ping(ctx: HttpContext) -> anyhow::Result<Response> {
    ///     let req_param = ctx.into_json::<ReqParam>().await?;
    ///     Resp::ok_with_empty()
    /// }
    /// ```
    pub async fn into_json<T: DeserializeOwned>(self) -> HttpResult<T> {
        match self.into_opt_json().await? {
            Some(v) => Ok(v),
            None => Err(HttpError::BodyNotJson),
        }
    }

    /// Asynchronous parsing of the body content of HTTP requests in JSON format,
    ///
    /// Returns:
    ///
    /// **Ok(Some(val))**: parse body success
    ///
    /// **Ok(None)**: body is empty
    ///
    /// **Err(e)**: parse error
    ///
    ///  ## Example
    /// ```rust
    /// use httpserver::{HttpContext, Response, Resp};
    ///
    /// #[derive(serde::Deserialize)]
    /// struct ReqParam {
    ///     user: Option<String>,
    ///     pass: Option<String>,
    /// }
    ///
    /// async fn ping(ctx: HttpContext) -> anyhow::Result<Response> {
    ///     let req_param = ctx.into_option_json::<ReqParam>().await?;
    ///     Resp::ok_with_empty()
    /// }
    /// ```
    pub async fn into_opt_json<T: DeserializeOwned>(self) -> HttpResult<Option<T>> {
        const MISSING_FIELD: &str = "missing field `";
        let id = self.id;

        let body = self.into_bytes().await?;

        let res = if body.has_remaining() {
            match serde_json::from_reader(body.reader()) {
                Ok(v) => Some(v),
                Err(e) => {
                    log_error!(id, "deserialize body to json fail: {e:?}");
                    let mut emsg = e.to_string();
                    //missing field `password`
                    if emsg.starts_with(MISSING_FIELD) {
                        let s = &emsg[MISSING_FIELD.len()..];
                        if let Some(pos) = s.find('`') {
                            emsg = format!("字段{}不能为空", &s[..pos]);
                        }
                    }
                    return Err(HttpError::Custom(emsg));
                }
            }
        } else {
            None
        };

        Ok(res)
    }

    pub async fn into_form_map(self) -> HttpResult<HashMap<CompactString, CompactString>> {
        Ok(form_urlencoded::parse(&self.into_bytes().await?)
            .into_iter()
            .fold(HashMap::new(), |mut init, v| {
                init.insert(CompactString::new(v.0), CompactString::new(v.1));
                init
            }))
    }

    pub async fn into_form_vec(self) -> HttpResult<Vec<(CompactString, CompactString)>> {
        Ok(form_urlencoded::parse(&self.into_bytes().await?)
            .into_iter()
            .fold(Vec::new(), |mut init, v| {
                init.push((CompactString::new(v.0), CompactString::new(v.1)));
                init
            }))
    }

    pub async fn into_bytes(self) -> HttpResult<Bytes> {
        match self.req.collect().await {
            Ok(v) => Ok(v.to_bytes()),
            Err(e) => {
                log_error!(self.id, "read request body fail: {e:?}");
                Err(HttpError::Custom(String::from("网络异常")))
            }
        }
    }

    pub fn get_url_params<'a>(&'a self) -> impl Iterator<Item = (Cow<'a, str>, Cow<'a, str>)> {
        form_urlencoded::parse(self.req.uri().query().unwrap_or("").as_bytes())
    }

    /// 获取客户端的真实ip, 获取优先级为X-Real-IP > X-Forwarded-For > socketaddr
    pub fn remote_ip(&self) -> Ipv4Addr {
        if let Some(ip) = self.req.headers().get("X-Real-IP") {
            if let Ok(ip) = ip.to_str() {
                if let Ok(ip) = ip.parse() {
                    return ip;
                }
            }
        }

        if let Some(ip) = self.req.headers().get("X-Forwarded-For") {
            if let Ok(ip) = ip.to_str() {
                if let Some(ip) = ip.split(',').next() {
                    if let Ok(ip) = ip.parse() {
                        return ip;
                    }
                }
            }
        }

        match self.addr.ip() {
            std::net::IpAddr::V4(ip) => ip,
            _ => std::net::Ipv4Addr::new(0, 0, 0, 0),
        }
    }

    pub fn header<K: AsHeaderName>(&self, key: K) -> Option<&HeaderValue> {
        self.req.headers().get(key)
    }

    pub fn attr<'a>(&'a self, key: &str) -> Option<&'a Value> {
        match &self.attrs {
            Some(atrr) => atrr.get(key),
            None => None,
        }
    }

    pub fn set_attr<T: Into<Value>>(&mut self, key: CompactString, value: T) {
        if self.attrs.is_none() {
            self.attrs = Some(HashMap::default());
        }
        self.attrs.as_mut().unwrap().insert(key, value.into());
    }

    pub fn user_id(&self) -> u32 {
        match self.uid.parse() {
            Ok(n) => n,
            Err(_) => 0,
        }
    }
}

impl Resp {
    /// Create a reply message with the specified status code and content
    ///
    /// Arguments:
    ///
    /// * `status`: http status code
    /// * `body`: http response body
    ///
    /// # Examples
    ///
    /// ```
    /// use httpserver::Resp;
    ///
    /// Resp::resp(hyper::StatusCode::Ok, hyper::Body::from(format!("{}",
    ///     serde_json::json!({
    ///         "code": 200,
    ///             "data": {
    ///                 "name":"kiven",
    ///                 "age": 48,
    ///             },
    ///     })
    /// ))?;
    /// ````
    pub fn resp<T: Into<Bytes>>(status: hyper::StatusCode, body: T) -> HttpResponse {
        hyper::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, APPLICATION_JSON)
            .body(Full::from(body.into()).boxed())
            .map_err(|e| HttpError::Custom(e.to_string()))
    }

    /// Create a reply with ApiResult
    ///
    /// Arguments:
    ///
    /// * `ar`: ApiResult
    ///
    pub fn resp_with<T: Serialize>(ar: &ApiResult<T>) -> HttpResponse {
        let status = if ar.is_ok() {
            hyper::StatusCode::OK
        } else {
            hyper::StatusCode::INTERNAL_SERVER_ERROR
        };

        let body = serde_json::to_vec(&ar.data).map_err(|e| HttpError::Custom(e.to_string()))?;

        hyper::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, APPLICATION_JSON)
            .body(Full::from(body).boxed())
            .map_err(|e| HttpError::Custom(e.to_string()))
    }

    /// Create a reply message with 200
    ///
    /// Arguments:
    ///
    /// * `body`: http response body
    ///
    /// # Examples
    ///
    /// ```
    /// use httpserver::Resp;
    ///
    /// Resp::resp_ok(hyper::Body::from(format!("{}",
    ///     serde_json::json!({
    ///         "code": 200,
    ///             "data": {
    ///                 "name":"kiven",
    ///                 "age": 48,
    ///             },
    ///     })
    /// ))?;
    /// ````
    pub fn resp_ok<T: Into<Bytes>>(body: T) -> HttpResponse {
        hyper::Response::builder()
            .header(CONTENT_TYPE, APPLICATION_JSON)
            .body(Full::from(body.into()).boxed())
            .map_err(|e| HttpError::Custom(e.to_string()))
    }

    /// Create a reply message with 200, response body is empty
    pub fn ok_with_empty() -> HttpResponse {
        Self::resp_ok(hyper::body::Bytes::from(r#"{"code":200}"#))
    }

    /// Create a reply message with 200
    ///
    /// Arguments:
    ///
    /// * `data`: http response for ApiResult.data
    ///
    /// # Examples
    ///
    /// ```
    /// use httpserver::Resp;
    ///
    /// Resp::ok(&serde_json::json!({
    ///     "code": 200,
    ///         "data": {
    ///             "name":"kiven",
    ///             "age": 48,
    ///         },
    /// }))?;
    /// ````
    #[inline]
    pub fn ok<T: ?Sized + Serialize>(data: &T) -> HttpResponse {
        // Self::ok_opt(Some(data))
        let mut w = Vec::with_capacity(512);
        w.extend_from_slice(br#"{"code":200,"data":"#);
        serde_json::to_writer(&mut w, data)
            .map_err(|_| HttpError::Custom(String::from("json序列化错误")))?;
        w.push(b'}');
        Self::resp_ok(w)
    }

    /// Create a reply message with 200
    ///
    /// Arguments:
    ///
    /// * `data`: http response for ApiResult.data
    ///
    #[inline]
    pub fn ok_opt<T: ?Sized + Serialize>(data: Option<&T>) -> HttpResponse {
        match data {
            Some(v) => Self::ok(v),
            None => Self::ok_with_empty(),
        }
    }

    /// Create a reply message with http status 500
    ///
    /// Arguments:
    ///
    /// * `message`: http error message
    ///
    /// # Examples
    ///
    /// ```
    /// use httpserver::Resp;
    ///
    /// Resp::fail("required field `username`")?;
    /// ````
    #[inline]
    pub fn fail(message: &str) -> HttpResponse {
        Self::fail_with_code(500, message)
    }

    /// Create a reply message with specified error code
    ///
    /// Arguments:
    ///
    /// * `code`: http error code
    /// * `message`: http error message
    ///
    /// # Examples
    ///
    /// ```
    /// use httpserver::Resp;
    ///
    /// Resp::fail_with_code(10086, "required field `username`")?;
    /// ````
    #[inline]
    pub fn fail_with_code(code: u32, message: &str) -> HttpResponse {
        Self::fail_with_status(hyper::StatusCode::INTERNAL_SERVER_ERROR, code, message)
    }

    /// Create a reply message with specified http status and error code
    ///
    /// Arguments:
    ///
    /// * `status`: http reponse status
    /// * `code`: http error code
    /// * `message`: http error message
    ///
    /// # Examples
    ///
    /// ```
    /// use httpserver::Resp;
    ///
    /// Resp::fail_with_status(hyper::StatusCode::INTERNAL_SERVER_ERROR,
    ///         10086, "required field `username`")?;
    /// ````
    pub fn fail_with_status(status: hyper::StatusCode, code: u32, message: &str) -> HttpResponse {
        let mut buf = itoa::Buffer::new();
        let code = buf.format(code);
        let mut w = Vec::with_capacity(256);
        w.extend_from_slice(br#"{"code":"#);
        w.extend_from_slice(code.as_bytes());
        w.extend_from_slice(br#","message":"#);
        serde_json::to_writer(&mut w, message).map_err(|e| HttpError::Custom(e.to_string()))?;
        w.push(b'}');
        Self::resp(status, w)
    }

    /// Create a reply message with specified http status and error code
    ///
    /// Arguments:
    ///
    /// * `status`: http reponse status
    /// * `code`: http error code
    /// * `message`: http error message
    ///
    #[inline]
    pub fn internal_server_error() -> HttpResponse {
        Self::fail("internal server error")
    }
}

#[async_trait::async_trait]
impl<F: Send + Sync + 'static, Fut> RunCallback for F
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = HttpResult<()>> + Send + 'static,
{
    async fn handle(self) -> HttpResult<()> {
        self().await
    }
}

/// Definition of callback functions for API interface functions
#[async_trait::async_trait]
impl<F: Send + Sync + 'static, Fut> HttpHandler for F
where
    F: Fn(HttpContext) -> Fut,
    Fut: std::future::Future<Output = HttpResponse> + Send + 'static,
{
    async fn handle(&self, ctx: HttpContext) -> HttpResponse {
        self(ctx).await
    }
}

impl<'a> Next<'a> {
    pub async fn run(mut self, ctx: HttpContext) -> HttpResponse {
        if let Some((current, next)) = self.next_middleware.split_first() {
            self.next_middleware = next;
            current.handle(ctx, self).await
        } else {
            (self.endpoint).handle(ctx).await
        }
    }
}

#[async_trait::async_trait]
impl HttpMiddleware for AccessLog {
    async fn handle<'a>(&'a self, mut ctx: HttpContext, next: Next<'a>) -> HttpResponse {
        let start = std::time::Instant::now();
        let ip = ctx.remote_ip();
        let id = ctx.id;
        let method = ctx.req.method().clone();
        let path = CompactString::new(ctx.req.uri().path());
        log_debug!(id, "{method} \x1b[33m{path}\x1b[0m");

        // 记录请求参数日志
        if log::log_enabled!(log::Level::Trace) {
            if let Some(query) = ctx.req.uri().query() {
                if !query.is_empty() {
                    log_trace!(id, "[QUERY] {}", query);
                }
            }
            for h in ctx.req.headers() {
                log_trace!(id, "[HEADER] {}: {}",
                    h.0.as_str(), std::str::from_utf8(h.1.as_bytes()).unwrap());
            }

            if let Some(ct) = ctx.req.headers().get(CONTENT_TYPE) {
                let ct = ct.as_bytes();
                if ct.starts_with(b"application/json") || ct.starts_with(b"application/x-www-form-urlencoded") {
                    let (parts, body) = ctx.req.into_parts();
                    let body = body.collect().await.unwrap().to_bytes();
                    log_trace!(id, "[BODY] {}", std::str::from_utf8(&body).unwrap());
                    ctx.req = Request::from_parts(parts, Full::from(body));
                }
            }
        }

        let mut res = next.run(ctx).await;
        // 输出接口调用耗时
        let ms = start.elapsed().as_millis();
        match &res {
            Ok(res) => {
                let c = ternary_expr!(res.status() == hyper::StatusCode::OK, 2, 1);
                log_info!(
                    id,
                    "{method} \x1b[34m{path} \x1b[3{c}m{}\x1b[0m {ms}ms, client: {ip}",
                    res.status().as_u16()
                );
            }
            Err(e) => log_error!(
                id,
                "{method} \x1b[34m{path}\x1b[0m \x1b[31m500\x1b[0m {ms}ms, error: {e:?}"
            ),
        };

        // 记录回复结果日志
        if log::log_enabled!(log::Level::Trace) {
            if let Ok(r) = res {
                let (parts, body) = r.into_parts();
                let body: Bytes = body.collect().await.unwrap().to_bytes();
                log_trace!(id, "[RESP] {}", std::str::from_utf8(&body).unwrap());
                res = Ok(Response::from_parts(parts, Full::from(body).boxed()));
            }
        }
        res
    }
}

impl HttpServer {
    /// Create a new HttpServer
    ///
    /// Arguments:
    ///
    /// * `use_access_log`: set Log middleware if true
    ///
    pub fn new(prefix: &str, use_access_log: bool) -> Self {
        let mut middlewares = Vec::<Box<dyn HttpMiddleware>>::new();
        if use_access_log {
            middlewares.push(Box::new(AccessLog));
        }

        HttpServer {
            prefix: CompactString::new(prefix),
            router: FnvHashMap::default(),
            middlewares,
            default_handler: Box::new(Self::handle_not_found),
            error_handler: Self::handle_error,
        }
    }

    /// set default function when no matching api function is found
    ///
    /// Arguments:
    ///
    /// * `handler`: The default function when no matching interface function is found
    ///
    pub fn default_handler(&mut self, handler: impl HttpHandler) {
        self.default_handler = Box::new(handler);
    }

    /// setup error handler for http server
    ///
    /// Arguments
    ///
    /// * `handler`: Exception event handling function
    pub fn error_handler(&mut self, handler: fn(id: u32, err: HttpError) -> Response) {
        self.error_handler = handler;
    }

    /// register api function for path
    ///
    /// Arguments:
    ///
    /// * `path`: api path
    /// * `handler`: handle of api function
    #[inline]
    pub fn register<S: AsRef<str>>(&mut self, path: S, handler: impl HttpHandler) {
        self.router
            .insert(CompactString::new(path), Box::new(handler));
    }

    /// register middleware
    pub fn middleware<T: HttpMiddleware>(&mut self, middleware: T) {
        self.middlewares.push(Box::new(middleware));
    }

    /// run http service and enter message loop mode
    ///
    /// Arguments:
    ///
    /// * `addr`: listen addr
    pub async fn run(self, addr: std::net::SocketAddr) -> HttpResult<()> {
        self.run_with_callbacck(addr, || async { Ok(()) }).await
    }

    /// run http service and enter message loop mode
    ///
    /// Arguments:
    ///
    /// * `addr`: listen addr
    /// * `f`: Fn() -> anyhow::Result<()>
    pub async fn run_with_callbacck<F: RunCallback>(
        self,
        addr: std::net::SocketAddr,
        f: F,
    ) -> HttpResult<()> {
        let listener = TcpListener::bind(addr).await?;
        f.handle().await?;
        self.log_server_info(addr);

        let srv_data = Arc::new(HttpServerData {
            id: AtomicU32::new(1),
            server: self,
        });

        loop {
            let (stream, addr) = listener.accept().await?;
            let io = TokioIo::new(stream);
            tokio::spawn(Self::on_accept(srv_data.clone(), addr, io));
        }
    }

    async fn on_accept(srv: Arc<HttpServerData>, addr: SocketAddr, io: TokioIo<TcpStream>) {
        let id = Self::step_id(&srv.id);

        let srv_fn = |req: hyper::Request<Incoming>| {
            let srv = srv.clone();
            async move {
                let path = req.uri().path();
                let endpoint = match srv.server.find_http_handler(path) {
                    Some(handler) => handler,
                    None => srv.server.default_handler.as_ref(),
                };
                let next = Next {
                    endpoint,
                    next_middleware: &srv.server.middlewares,
                };

                let (parts, body) = req.into_parts();
                let body = Full::new(body.collect().await.unwrap().to_bytes());
                let req = Request::from_parts(parts, body);

                let ctx = HttpContext {
                    req,
                    addr,
                    id,
                    uid: CompactString::with_capacity(0),
                    attrs: None,
                };

                let resp = match next.run(ctx).await {
                    Ok(resp) => resp,
                    Err(e) => (srv.server.error_handler)(id, e),
                };

                Ok::<_, Infallible>(resp)
            }
        };

        if let Err(err) = http1::Builder::new()
            .serve_connection(io, service::service_fn(srv_fn))
            .await
        {
            log_error!(id, "http process error: {err:?}");
        }
    }

    fn step_id(id: &AtomicU32) -> u32 {
        let curr_id = id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if curr_id == 0 {
            id.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        } else {
            curr_id
        }
    }

    fn find_http_handler(&self, path: &str) -> Option<&dyn HttpHandler> {
        // 前缀不匹配
        if !self.prefix.is_empty() && !path.starts_with(self.prefix.as_str()) {
            return None;
        }

        // 找到直接匹配的路径
        let mut path = CompactString::new(&path[self.prefix.len()..]);
        if let Some(handler) = self.router.get(&path) {
            return Some(handler.as_ref());
        }

        // 尝试递归上级路径查找带路径参数的接口
        while let Some(pos) = path.rfind('/') {
            path.truncate(pos + 1);
            path.push('*');

            if let Some(handler) = self.router.get(&path) {
                return Some(handler.as_ref());
            }

            path.truncate(path.len() - 2);
        }

        None
    }

    fn handle_error(id: u32, err: HttpError) -> Response {
        match Resp::fail(&err.to_string()) {
            Ok(val) => val,
            Err(e) => {
                log_error!(id, "handle_error except: {e:?}");
                let body = Full::from("internal server error").boxed();
                let mut res = hyper::Response::new(body);
                *res.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
                res
            }
        }
    }

    async fn handle_not_found(_: HttpContext) -> HttpResponse {
        Ok(Resp::fail_with_status(
            hyper::StatusCode::NOT_FOUND,
            404,
            "Not Found",
        )?)
    }

    fn log_server_info(&self, addr: SocketAddr) {
        if !log::log_enabled!(log::Level::Trace) {
            return;
        }

        let mut buf = String::with_capacity(1024);
        buf.push_str("Registered interface:");
        let buf = self.router.iter().fold(buf, |mut buf, v| {
            buf.push('\n');
            buf.push('\t');
            buf.push_str(v.0.as_str());
            buf
        });
        log::trace!("{}", buf);

        log::info!("Startup http server on \x1b[34m{addr}\x1b[0m");
    }
}
