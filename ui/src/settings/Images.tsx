/* Settings, Images: whether agents may make images with Gemini, and how textures are made (a Gemini key, if any). */
import { useState } from "preact/hooks";
import { api } from "../api";
import { useStore } from "../store";
import { host } from "../host";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Switch } from "../ui/Switch";
import { Field } from "../ui/Field";

export function Images() {
  const status = useStore((s) => s.status), [key, setKey] = useState("");
  const set = async (body: unknown) => { await api("/api/gemini", body); await host.refresh(); };
  const how = ((status.ai || {}).installed || {}).gemini ? "Made through Antigravity on your own Google account, at no extra cost. If that fails, " + (status.gemini_api ? "your API key is used." : "you get the prompt to run in Gemini yourself.")
    : status.gemini_api ? "Made automatically with your API key. Google charges for each one." : "Made through your own Gemini chat, at no extra cost." + (status.gemini ? " A key is saved but switched off." : "");
  const known = "ai" in status || "gemini" in status;
  return (
    <>
      <Group id="set-ai-images" title="AI images">
        <SettingsRow id="set-gem-images" find="icons thumbnail art gemini antigravity" title="Agents may generate images with Gemini" labelFor="gem-images" desc="Icons, thumbnail art and UI art, each using some of your Gemini plan. Off: agents ask you first."
          more="Icons, thumbnail art and UI art are made through Antigravity, and each picture uses some of your Gemini plan. One chat may start 12 an hour and one project's team 30, textures included. Off: agents ask you first."
          ctl={<Switch id="gem-images" on={status.gemini_images !== false} onChange={(on) => set({ images: on })} />} />
      </Group>
      <Group id="set-textures" title="Textures">
        <SettingsRow id="set-texture-how" title="How textures are made" desc={known ? how : ""} descId="gem-state"
          more="Without Antigravity, you get the prompt, make the image in your own Gemini chat, and it is picked up when you copy it. A key is only used if you switch it on below; Google then charges about 3 cents an image. The /texture, /particle and /decal commands always use your own chat. Textures only: never UI, icons or lettering. Cursor chats use only a key saved here, not one set in your computer's environment." />
        <SettingsRow id="set-gem-key" find="api key google" title="Gemini API key" labelFor="gem-key" desc="Only used if you switch it on below."
          ctl={<Field id="gem-key" type="password" value={key} onInput={setKey} placeholder="Gemini API key" label="Gemini API key">
            <button class="pill" id="gem-save" onClick={async () => { const k = key.trim(); if (!k) return; await api("/api/gemini", { key: k }); setKey(""); host.refresh(); }}>Save</button>
            <button class="pill" id="gem-clear" style={known && !status.gemini ? "display: none;" : undefined} onClick={() => set({ key: "" })}>Remove key</button></Field>} />
        <SettingsRow id="set-gem-api" title="Use my API key" desc="Google charges for each image, about 3 cents."
          ctl={<Switch id="gem-api" label="Use my API key (Google charges per image)" hidden={known && !status.gemini} on={!!status.gemini_api} onChange={(on) => set({ use_api: on })} />} />
      </Group>
    </>
  );
}
