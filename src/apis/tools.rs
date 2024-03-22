//! 实用工具接口

use crate::{convert, AppGlobal};
use compact_str::{format_compact, CompactString, ToCompactString};
use httpserver::{log_info, HttpContext, HttpErr, HttpResponse, Resp};
use localtime::LocalTime;
use serde::{Deserialize, Serialize};

/// 服务测试，测试服务是否存活
pub async fn ping(ctx: HttpContext) -> HttpResponse {
    #[derive(Deserialize)]
    struct Req {
        reply: Option<CompactString>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Res {
        reply: CompactString,
        now: LocalTime,
        server: CompactString,
    }

    let reply = match ctx.into_opt_json::<Req>().await? {
        Some(ping_params) => ping_params.reply,
        None => None,
    }
    .unwrap_or(CompactString::new("pong"));

    Resp::ok(&Res {
        reply,
        now: LocalTime::now(),
        server: format_compact!("{}/{}", crate::APP_NAME, crate::APP_VER),
    })
}

/// 服务状态
pub async fn status(ctx: HttpContext) -> HttpResponse {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Res {
        startup:      LocalTime,    // 服务启动时间
        resp_count:   u32,          // 总响应次数
        context_path: &'static str, // 服务上下文
        app_name:     &'static str, // 应用名称
        app_ver:      &'static str, // 应用版本
    }

    let app_global = AppGlobal::get();

    Resp::ok(&Res {
        startup: LocalTime::from_unix_timestamp(app_global.startup_time),
        resp_count: ctx.id,
        context_path: crate::SERVICE_PREFIX,
        app_name: crate::APP_NAME,
        app_ver: crate::APP_VER,
    })
}

/// 获取客户端ip
pub async fn ip(ctx: HttpContext) -> HttpResponse {
    #[derive(Serialize)]
    // #[serde(rename_all = "camelCase")]
    struct Res {
        ip: CompactString,
    }

    let ip = ctx.remote_ip().to_compact_string();

    Resp::ok(&Res { ip })
}

/// 查询坐标所在行政区域
pub async fn area(ctx: HttpContext) -> HttpResponse {
    #[derive(Deserialize)]
    struct Req {
        lng: f32,
        lat: f32,
    }

    // type Res = convert::RegionInfo;

    let rid = ctx.id;
    let param = ctx.into_json::<Req>().await?;
    log_info!(rid, "坐标查询: {}, {}", param.lng, param.lat);

    let region_info = convert::resolve_region(param.lng, param.lat)
        .http_err(rid)?;

    if let Some(ri) = region_info {
        Resp::ok(&ri)
    } else {
        Resp::fail("未找到坐标对应的行政区域信息")
    }

}
