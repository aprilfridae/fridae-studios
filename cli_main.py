#!/usr/bin/env python
"""fridae-cli: the Python tools behind Fridae Studios.

    fridae-cli.exe --mcp       MCP server (what agents connect to)
    fridae-cli.exe --rbx ...   command line (fridae-cli.exe --rbx doctor)

The window itself is the Rust program in app-rs (Fridae Studios.exe).
"""
import os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


def main():
    if "--mcp" in sys.argv[1:2]:
        import rbxkit_mcp
        rbxkit_mcp.main()
    elif "--rbx" in sys.argv[1:2]:
        import rbx
        rbx.main(sys.argv[2:])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
