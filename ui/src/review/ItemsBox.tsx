/* The chosen project's things in one box: those with pictures or a clip on one side, the rest on the other. With
   only one kind, that side is the whole box. */
import type { Thing } from "./model";
import type { Answer } from "./Review";
import { PictureItem } from "./PictureItem";
import { PlainItem } from "./PlainItem";

export function ItemsBox({ things, labelledBy, flash, onAnswered }: { things: Thing[]; labelledBy?: string; flash: string; onAnswered: Answer }) {
  const pics = things.filter((t) => t.media.length), plain = things.filter((t) => !t.media.length);
  return (
    <div id="rv-items" role="tabpanel" tabIndex={-1} aria-labelledby={labelledBy} class={pics.length && plain.length ? "two" : undefined} hidden={!things.length}>
      <div class="rv-col" id="rv-pics" hidden={!pics.length}>
        {pics.map((t) => <PictureItem key={t.key} thing={t} flash={flash === t.key} onAnswered={onAnswered} />)}
      </div>
      <div class="rv-col" id="rv-plain" hidden={!plain.length}>
        {plain.map((t) => <PlainItem key={t.key} thing={t} flash={flash === t.key} onAnswered={onAnswered} />)}
      </div>
    </div>
  );
}
