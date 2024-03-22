# renren_rust
基于gcj-02坐标系的反向查询所属省市区服务

附带的china-region-20190902.7z是一个包含国内所有省市区的geojson文件

---

#### 项目地址
<https://github.com/kivensoft/geo_server>

###### 主要的第三方依赖库
- rust 1.65+ 媲美C语言的强类型开发语言
- tokio 1.26+ 目前最流行也是性能最好的异步io运行时
- hyper 1.0+ http底层协议库，是众多三方web框架使用的基础库
- serde_json 1.0+ 最流行也是速度最快的json序列化库
- anyhow 1.0+ 最流行的错误处理库，增强标准库的错误处理
- log 0.4+ 日志门面库，rust标准库
- memmap 0.7+ 内存映射文件库，跨平台
- persy 1.5+ 纯rust实现的单文件BTree数据库
- geo 0.28+ 地理信息处理库
- asynclog 简单的异步日志库，采用独立线程进行日志输出
- appconfig 命令行参数及配置文件参数解析库

---

###### 源代码下载
`git clone git@github.com/kivensoft/geo_server.git`

###### 编译
`cargo build`

###### 解压china-region-20190902.7z
`7z x china-region-20190902.7z`

###### 生成persy数据库
`geo_server --convert china-region.json`

###### 运行服务
`geo_server`
