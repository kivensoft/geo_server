@echo off
rem 目录定时同步脚本
set project=geo_server
set src=/f/%project%/
set dst=/e/projects/renren/%project%/
echo Synchronize folders:
echo    %src%  ^<=^>  %dst%
set /p var="Are you sure?(y/n)"
if %var%==n goto end
:loop
rsync -av --delete --exclude="/target/" --exclude="/.git/" %src% %dst%
echo waiting for synchronize folders %src%
timeout /t 600
goto loop
:end
