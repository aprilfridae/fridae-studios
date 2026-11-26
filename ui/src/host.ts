/* What the new screens ask of the old page (ui/index.html) while both exist: the old page hands these over once with
   fridae.setHost({...}). Each is a function the old page already had; a screen never reaches into the old page any
   other way. */
export type Host = {
  refresh(): Promise<void>;                       /* ask the app's status again (and put it in the store) */
  closeSetup(): void; closeUsage(): void; openUsage(): void; openSetup(page?: string): void;
  panelOpened(first: Element | null): void;       /* a window opened over the page: the keyboard goes to `first` */
  say(text: string): void;                        /* one line over the message box */
  anyBusy(): boolean;                             /* some chat is working */
  example(text: string): void;                    /* a Guide row: close Settings and put its example in the message box (or open its page) */
  allowCommands(names: string[], allow: boolean): Promise<boolean>;
  ghAct(action: string): Promise<any>;            /* a press on GitHub's row */
  engineInstall(): void;
  reloadSaved(): Promise<void>;
  setScale(v: number): void; setSideW(w: number): void; setTheme(next: { id: string; mode: string }): void;
  themes(): { THEMES: any; ORDER: string[]; colors(id: string, appearance: string): any };
  swarmCeiling(n: number): void;                  /* Most experts at once was changed */
  skillsCount(): void;                            /* the count of open decisions changed (the Overview shows it too) */
};
const missing = new Proxy({}, { get: () => () => undefined }) as Host;
export let host: Host = missing;
export function setHost(h: Host) { host = h; }
