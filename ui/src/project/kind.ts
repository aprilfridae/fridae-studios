/* A project's kind and the project in front, read from the store the same way the old page reads them. */
import { store, type Project } from "../store";

/* a Roblox game: marked so, or the app found a place, a Rojo project or Luau scripts in its folder */
export const isGame = (pr: Project | null | undefined) => !!pr && ["game", "roblox"].includes((pr.type || pr.kind) as string);
export const projOf = (path: string) => store.get().projects.find((x) => x.path === path);
export const isGameProj = (path: string | null | undefined) => !!path && isGame(projOf(path));
/* the pages of no one project: Overview, Review and New project */
export const isPage = (v: string) => v === "newproj" || v === "all" || v === "review";
/* "H:/games/obby-rush" -> "obby-rush" */
export const baseName = (p: string) => p.split(/[\/]/).filter(Boolean).pop() || p;
/* the project in front, as the app's list has it */
export function frontProj(): Project | null {
  const path = store.get().nav.proj;
  if (!path) return null;
  return projOf(path) || { path, name: baseName(path) };
}
