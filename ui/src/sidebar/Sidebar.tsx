/* The sidebar: the Fridae mark, the list (List.tsx), the foot (Foot.tsx), and its right edge, which is a handle
   (Grip.tsx). The project's menu, the dialog that asks before a project goes and the pairing dialog (ui/src/remote)
   are drawn over the page (SideOver), outside the part of it that goes inert under a window. */
import { useEffect } from "preact/hooks";
import { useStore } from "../store";
import { DeleteProject } from "./DeleteProject";
import { Foot, UsePop } from "./Foot";
import { Grip } from "./Grip";
import { List } from "./List";
import { ProjectMenu } from "./ProjectMenu";
import { RenameBox } from "./Rename";
import "./sidebar.css";
import { Pair } from "../remote/Pair";

export function Sidebar() {
  const busy = useStore((s) => s.chats.some((p) => p.busy));
  /* the logo turns while anything works */
  useEffect(() => { document.body.classList.toggle("working", busy); }, [busy]);
  return (
    <>
      <span class="mark" data-drag><img src="/logo.png" alt="" /><span>Fridae</span></span>
      <List />
      <Foot />
      <RenameBox />
      <Grip />
      <UsePop />
    </>
  );
}
export function SideOver() {
  return <><ProjectMenu /><DeleteProject /><Pair /></>;
}
