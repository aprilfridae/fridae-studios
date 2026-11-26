/* What the new screens ask of the old page (ui/index.html) while both exist: the old page hands these over once with
   fridae.setHost({...}). Each is a function the old page already had; a screen never reaches into the old page any
   other way. */
export type Host = {
  refresh(): Promise<void>;                       /* ask the app's status again (and put it in the store) */
  panelOpened(first: Element | null): void;       /* a window opened over the page: the keyboard goes to `first` */
  panelClosed(): void;                            /* and closed: the keyboard goes back, unless another is still open */
  closePops(): void; closeProject(): void;        /* the message box's menus, and the project's panel */
  loadEngine(): void; loadGithub(): void;         /* what Settings shows of the promo engine and GitHub is asked for again */
  /* what the sidebar and Overview press */
  showAll(): void; showNewProject(): void; setView(view: string): void;
  showProject(path: string, tab?: string): void;  /* a project comes to the front, on its last tab ("" is No project) */
  openChat(id: string): void;                     /* an open chat comes to the front */
  openChatById(id: string): void;                 /* the same for any chat: a saved one is opened first */
  openInReview(path: string, id?: string): void;
  projectRenamed(path: string, name: string): Promise<void>;
  ink(): string;                                  /* the text colour as "r,g,b", for what is drawn on a canvas */
  projectDeleted(path: string): Promise<void>;    /* it left the list: the page goes elsewhere if it was in front */
  say(text: string): void;                        /* one line over the message box */
  anyBusy(): boolean;                             /* some chat is working */
  example(text: string): void;                    /* a Guide row: close Settings and put its example in the message box (or open its page) */
  allowCommands(names: string[], allow: boolean): Promise<boolean>;
  ghAct(action: string): Promise<any>;            /* a press on GitHub's row */
  engineInstall(): void;
  reloadSaved(): Promise<void>;
  setScale(v: number): void; setTheme(next: { id: string; mode: string }): void;
  themes(): { THEMES: any; ORDER: string[]; colors(id: string, appearance: string): any };
  swarmCeiling(n: number): void;                  /* Most experts at once was changed */
};
const missing = new Proxy({}, { get: () => () => undefined }) as Host;
export let host: Host = missing;
export function setHost(h: Host) { host = h; }
