@echo off
rem 目录定时同步脚本
set project=geo_server
set src=/f/%project%/
set dst=/e/projects/renren/%project%/

rsync -av --exclude="/target/" --exclude="/.git/" %dst% %src%
