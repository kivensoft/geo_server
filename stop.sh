#!/bin/sh
PROG=geo_server
echo 查找${PROG}进程...
RID=`pidof $PROG`
if [ "$RID" != "" ];then
    echo 找到${PROG}进程ID: $RID
    kill -9 $RID
    echo 终止进程[$RID]成功
else
    echo ${PROG}进程不存在
fi
