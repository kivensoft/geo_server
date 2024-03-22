#!/bin/sh
nohup chroot /app/geo_server /geo_server > console.log 2>&1 &
echo 启动服务geo_server完成
