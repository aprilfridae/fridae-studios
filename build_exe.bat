@echo off
rem Builds Fridae Studios into dist\Fridae Studios (needs Rust, and: python -m pip install pyinstaller numpy pillow opencv-python)
cd /d "%~dp0"
python build_app.py %*
