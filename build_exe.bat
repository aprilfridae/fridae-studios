@echo off
rem Builds Fridae Studios into dist\Fridae Studios (needs Rust, and: python -m pip install pyinstaller numpy pillow opencv-python "fastembed==0.9.*" "qdrant-client==1.20.*")
cd /d "%~dp0"
python build_app.py %*
