"""`rbx rag ...`: the knowledge index's service, setup steps and check. Argument parsing and printing only.

  rag serve --mode light|heavy --data DIR --port N --token-file PATH [--qdrant-url URL --qdrant-key-file PATH]
            [--exit-with-stdin]
  rag setup STEP --mode light|heavy --data DIR [--json-progress]
  rag check --mode light|heavy --data DIR [--json]

Secrets (the service token, Qdrant's key) are read from files, never taken on the command line, and never printed.
"""
import json
import sys


def _say(line):
    print(line, flush=True)


def _serve(a):
    from rbxrag import serve
    serve.run(a.mode, a.data, a.port, a.token_file, a.qdrant_url, a.qdrant_key_file, a.ollama_url,
              exit_with_stdin=a.exit_with_stdin)


def _setup(a):
    from rbxrag import setup
    hooks = {"url": a.ollama_url} if a.ollama_url else {}
    if a.json_progress:
        done = setup.run_cli(a.step, a.mode, a.data, _say, **hooks)
    else:
        seen = {}

        def progress(step, fraction):
            tenth = int(fraction * 10)
            if seen.get(step) != tenth:
                seen[step] = tenth
                _say("%s: %d%%" % (step, int(fraction * 100)))
        done = setup.run_step(a.step, a.mode, a.data, progress, **hooks)
        _say("ready: %s" % str(done["ready"]).lower())
        if done["error"]:
            print(done["error"], file=sys.stderr, flush=True)
    sys.exit(0 if done["ready"] else 1)


def _check(a):
    import os
    os.environ.setdefault("HF_HUB_OFFLINE", "1")  # a check never downloads
    from rbxrag import setup
    st = setup.check(a.mode, a.data, **({"url": a.ollama_url} if a.ollama_url else {}))
    if a.json:
        _say(json.dumps(st))
    else:
        for k, v in st.items():
            _say("%s: %s" % (k, v))


def register(sub):
    p = sub.add_parser("rag", help="the knowledge index: run its service, its setup steps, check what is set up")
    s = p.add_subparsers(dest="rag_cmd", required=True)

    def common(x):
        x.add_argument("--mode", choices=("light", "heavy"), required=True)
        x.add_argument("--data", required=True, help="the index's folder (the app's own rag folder in Application Support)")
        x.add_argument("--ollama-url", help=argparse_hidden())

    x = s.add_parser("serve", help="answer searches on 127.0.0.1 (prints {\"listening\": port} when bound)")
    common(x)
    x.add_argument("--port", type=int, default=0)
    x.add_argument("--token-file", required=True, help="file holding the bearer token")
    x.add_argument("--qdrant-url", help="Heavy: the Qdrant service the app started")
    x.add_argument("--qdrant-key-file", help="Heavy: file holding Qdrant's API key")
    x.add_argument("--exit-with-stdin", action="store_true",
                   help="end when stdin closes (the app passes a pipe, so the service never outlives it)")
    x.set_defaults(fn=_serve)

    x = s.add_parser("setup", help="run one setup step: models, ollama-install, ollama-pull, qdrant-install")
    x.add_argument("step", choices=("models", "ollama-install", "ollama-pull", "qdrant-install"))
    common(x)
    x.add_argument("--json-progress", action="store_true", help="one JSON line per progress step, for the app")
    x.set_defaults(fn=_setup)

    x = s.add_parser("check", help="what is set up (no network except the local Ollama)")
    common(x)
    x.add_argument("--json", action="store_true")
    x.set_defaults(fn=_check)


def argparse_hidden():
    import argparse
    return argparse.SUPPRESS
