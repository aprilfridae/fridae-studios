/* A thing to look at: its pictures or clip, all one size (a picture opens large when pressed), its title, and
   Approve or Ask for changes. */
import { Card, Decide, type ItemProps } from "./Card";
import { isClip, mediaUrl } from "./model";
import { zoom } from "./Lightbox";
import { store } from "../store";

export function PictureItem({ thing, flash, onAnswered }: ItemProps) {
  const it = thing.item!, r = store.get().review[thing.path];
  return (
    <Card thing={thing} flash={flash}>{(kept, keep) => (<>
      <div class={"rv-media" + (thing.media.length > 1 ? " many" : "")}>
        {thing.media.map((f) => {
          const src = mediaUrl(r, it.id, f);
          if (isClip(f)) return <video key={f} controls playsInline preload="metadata" src={src} />;
          return (
            <button key={f} aria-label={"See " + f + " large"} onClick={(e) => zoom(src, thing.pics, e.currentTarget as HTMLElement)}>
              <img alt="" loading="lazy" src={src} />
            </button>
          );
        })}
      </div>
      <Decide thing={thing} kept={kept} keep={keep} onAnswered={onAnswered} />
    </>)}</Card>
  );
}
