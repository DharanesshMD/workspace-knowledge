#!/bin/sh
# One demo project folder per supported coding agent, set up from the server's own agent-setup endpoint.
# All the work (and the documentation) is in make_demo_folders.py next to this file.
exec python3 "$(dirname "$0")/make_demo_folders.py" "$@"
