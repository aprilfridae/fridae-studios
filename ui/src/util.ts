/* Small things every screen words the same way. */
export function ago(ts: number) {
  const s = Date.now() / 1000 - ts;
  if (s < 90) return "just now";
  if (s < 3600) return Math.round(s / 60) + " min ago";
  if (s < 86400) return Math.round(s / 3600) + " h ago";
  if (s < 86400 * 7) return Math.round(s / 86400) + " d ago";
  return new Date(ts * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}
export function fmtTokens(n: number) { return n >= 1e6 ? (n / 1e6).toFixed(n >= 1e7 ? 0 : 1) + "M" : n >= 1e3 ? Math.round(n / 1e3) + "k" : String(n); }
/* the same POST, with "could not reach the app" as an answer instead of a throw */
import { api } from "./api";
export async function post(path: string, body?: unknown): Promise<any> {
  try { return await api(path, body); } catch (e) { return { ok: false, error: "Could not reach the app." }; }
}
export async function quiet(path: string, body?: unknown): Promise<any> { try { return await api(path, body); } catch (e) { return null; } }
