mod apis;
mod convert;
mod cors;

use std::fmt::Write;
use httpserver::HttpServer;
use localtime::LocalTime;
use smallstr::SmallString;
use tokio::signal;

const BANNER: &str = r#"
   ____ ____  ____     ________  ______   _____  _____
  / __ `/ _ \/ __ \   / ___/ _ \/ ___/ | / / _ \/ ___/
 / /_/ /  __/ /_/ /  (__  )  __/ /   | |/ /  __/ /
 \__, /\___/\____/  /____/\___/_/    |___/\___/_/
/____/  kivensoft #
"#;

const APP_NAME: &str = "renren_rust";
/// app版本号, 来自编译时由build.rs从cargo.toml中读取的版本号(读取内容写入.version文件)
const APP_VER: &str = include_str!(concat!(env!("OUT_DIR"), "/.version"));
const SERVICE_PREFIX: &str = "/api/";

appconfig::appglobal_define!{app_global, AppGlobal,
    startup_time: i64,
}

appconfig::appconfig_define!{app_conf, AppConf,
    log_level   : String => ["L",  "log-level",    "LogLevel",          "日志级别(trace/debug/info/warn/error/off)"],
    log_file    : String => ["F",  "log-file",     "LogFile",           "日志的相对路径或绝对路径文件名"],
    log_max     : String => ["M",  "log-max",      "LogFileMaxSize",    "日志文件的最大长度 (单位: k|m|g)"],
    log_no_async: bool   => ["",   "log-no-async", "",                  "禁用异步日志"],
    no_console  : bool   => ["",   "no-console",   "",                  "禁止将日志输出到控制台"],
    database    : String => ["d",  "database",     "Database",          "指定的persy数据库文件名"],
    convert     : String => ["",   "convert",      "Convert",           "指定要转换为persy格式的json格式的文件名"],
    threads     : String => ["t",  "threads",      "Threads",           "设置应用的线程数"],
    listen      : String => ["l",  "listen",       "Listen",            "服务监听端点 (ip地址:端口号)"],
}

impl Default for AppConf {
    fn default() -> Self {
        Self {
            log_level:      String::from("info"),
            log_file:       String::with_capacity(0),
            log_max:        String::from("10m"),
            log_no_async:   false,
            no_console:     false,
            database:       String::from("china_region.persy"),
            convert:        String::with_capacity(0),
            threads:        String::from("0"),
            listen:         String::from("127.0.0.1:6406"),
        }
    }
}

macro_rules! arg_err {
    ($text:literal) => {
        concat!("参数 ", $text, " 格式错误")
    };
}

/// 初始化命令行参数和配置文件
fn init() -> Option<(&'static mut AppConf, &'static mut AppGlobal)> {
    let mut buf = SmallString::<[u8; 512]>::new();

    write!(buf, "{APP_NAME} 版本 {APP_VER} 版权所有 Kivensoft 2021-2023.").unwrap();
    let version = buf.as_str();

    // 从命令行和配置文件读取配置
    let ac = AppConf::init();
    if !appconfig::parse_args(ac, version).expect("解析参数失败") {
        return None;
    }

    // 初始化全局常量参数
    let ag = AppGlobal::init(AppGlobal {
        startup_time: LocalTime::now().timestamp(),
    });

    // if ac.listen.starts_with("0.0.0.0:") {
    //     panic!("arg listen 必须指定具体的ip而不能使用`0.0.0.0`")
    // }
    if !ac.listen.is_empty() && ac.listen.as_bytes()[0] == b':' {
        ac.listen.insert_str(0, "127.0.0.1");
    };

    // 初始化日志组件
    let log_level = asynclog::parse_level(&ac.log_level).expect(arg_err!("log-level"));
    let log_max = asynclog::parse_size(&ac.log_max).expect(arg_err!("log-max"));

    if log_level == log::Level::Trace {
        log::trace!("配置详情: {ac:#?}\n");
    }

    asynclog::init_log(log_level, ac.log_file.clone(), log_max,
        !ac.no_console, !ac.log_no_async).expect("初始化日志组件失败");
    asynclog::set_level("mio".to_owned(), log::LevelFilter::Info);
    asynclog::set_level("hyper".to_owned(), log::LevelFilter::Info);
    asynclog::set_level("want".to_owned(), log::LevelFilter::Info);
    asynclog::set_level("tokio_util".to_owned(), log::LevelFilter::Info);

    // 工具参数处理
    if !ac.convert.is_empty() {
        convert::convert(ac);
        return None;
    }

    // 在控制台输出logo
    if let Some((s1, s2)) = BANNER.split_once('%') {
        // let s2 = &s2[APP_VER.len() - 1..];
        buf.clear();
        write!(buf, "{s1}{APP_VER}{s2}").unwrap();
        appconfig::print_banner(&buf, true);
    }

    Some((ac, ag))
}

/// 创建http服务接口
fn reg_apis(srv: &mut HttpServer) {
    srv.register("ping", apis::tools::ping);
    srv.register("status", apis::tools::status);
    srv.register("ip", apis::tools::ip);
    srv.register("area", apis::tools::area);
}

// #[tokio::main(worker_threads = 4)]
// #[tokio::main(flavor = "current_thread")]
fn main() {
    let (ac, _ag) = match init() {
        Some((ac, ag)) => (ac, ag),
        None => return,
    };
    log::info!("正在启动服务...");

    // 初始化数据库
    convert::init_db().unwrap();

    let threads = ac.threads.parse::<usize>().expect(arg_err!("threads"));
    let addr: std::net::SocketAddr = ac.listen.parse().expect(arg_err!("listen"));
    let mut srv = HttpServer::new(SERVICE_PREFIX, true);

    srv.middleware(cors::CorsMiddleware);
    reg_apis(&mut srv);

    let addr_clone = addr;
    let async_fn = async move {
        // 设置权限校验中间件
        // auth::init().await;
        // srv.middleware(auth::Authentication);

        // 运行http服务
        tokio::spawn(async move {
            if let Err(e) = srv.run_with_callbacck(addr, move || async move {
                log::info!("服务器启动成功: {}", addr_clone);
                Ok(())
            }).await {
                log::error!("启动http服务失败: {e:?}");
            }
        });

        // 监听ctrl+c事件
        match signal::ctrl_c().await {
            Ok(()) => {
                log::info!("关闭{APP_NAME}服务");
            },
            Err(e) => {
                log::error!("Unable to listen for shutdown signal: {e:?}");
            },
        }
    };


    #[cfg(not(feature = "multi_thread"))]
    {
        assert!(threads == 1, "{APP_NAME} 当前版本是单线程版本");

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async_fn);
    }

    #[cfg(feature = "multi_thread")]
    {
        assert!(threads <= 256, "线程数量范围在 0-256 之间");

        let mut builder = tokio::runtime::Builder::new_multi_thread();
        if threads > 0 {
            builder.worker_threads(threads);
        }

        builder.enable_all()
            .build()
            .unwrap()
            .block_on(async_fn)
    }

}
