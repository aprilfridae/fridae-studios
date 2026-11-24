"""Network helpers. Every call to a service on this Mac (the RAG service, Ollama) goes through LOOPBACK, which never
uses an HTTP proxy: urllib otherwise honours http_proxy and the system proxy even for 127.0.0.1, and the proxy would
receive tokens and note text. Downloads from the internet use the normal opener (a user's proxy applies there)."""
import urllib.request

LOOPBACK = urllib.request.build_opener(urllib.request.ProxyHandler({}))
