#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
uv venv --python 3.12 "$script_directory/.venv"
uv pip install --python "$script_directory/.venv/bin/python" "nautilus_trader==2.0.0rc2"
