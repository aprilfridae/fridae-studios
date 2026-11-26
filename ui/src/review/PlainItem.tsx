/* A thing with no picture: finished work to approve, or a to-do only the user can do. */
import { Card, Decide, DoneButton, type ItemProps } from "./Card";

export function PlainItem({ thing, flash, onAnswered }: ItemProps) {
  return (
    <Card thing={thing} flash={flash}>
      {(kept, keep) => (thing.item ? <Decide thing={thing} kept={kept} keep={keep} onAnswered={onAnswered} /> : <DoneButton thing={thing} kept={kept} keep={keep} onAnswered={onAnswered} />)}
    </Card>
  );
}
