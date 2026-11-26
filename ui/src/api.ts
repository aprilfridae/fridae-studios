/* The app's own server: the same call the old page makes (its `api()`), with the window's token. */
export const TOKEN = new URLSearchParams(location.search).get("t") || "";

export async function api(path: string, body?: unknown): Promise<any> {
  const r = await fetch(path, { method: body === undefined ? "GET" : "POST", headers: { "X-Token": TOKEN, "Content-Type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  return r.json();
}
