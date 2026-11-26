/* What the chat asks of the old page (ui/index.html) while both exist: the page's own state (the chats, the project
   and the tab in front, the app's status, each project's Review reply) and the functions it already had, handed over
   once with fridae.chat.init({...}). The values that change are read through getters, so `H.current` is always the
   page's own `current`. */
export let H: any = {};
export function setChatHost(h: any) { H = h; }
