// A picture in a conversation, drawn: from its bytes when the entry has them,
// and asked of the host the first time it is drawn when it does not.
//
// The pane opens its channel saying it will ask (`fetch_pictures`), so the
// replay leaves every picture's bytes out and puts the number to ask by in
// their place. A session that takes screenshots is mostly pictures, and a
// conversation opens with its earlier turns folded shut, so most of them are
// never drawn at all. An entry that arrives as it happens has its pictures
// with it, since that is the one being watched.

import { createContext, useContext, useEffect, useState } from "react";

import type { ChatFile } from "../gen/ChatFile";
import type { Picture } from "../gen/Picture";
import { Zoomable } from "../Picture";

/// A picture as the transcript has it: its bytes, or, from a replay that held
/// them back, the entry it is in and its place there.
export type ChatPicture = { mime: string; data: string } | { mime: string; seq: number; index: number };

/// The images in a tool result's content: whole, or held back.
export function resultPictures(content: unknown, seq: number): ChatPicture[] {
  if (!Array.isArray(content)) return [];
  return content.flatMap((part): ChatPicture[] => {
    if (part?.type !== "image") return [];
    const source = part.source as { type?: string; media_type?: string; data?: string; index?: number } | undefined;
    if (!source?.media_type) return [];
    if (source.type === "base64" && source.data) return [{ mime: source.media_type, data: source.data }];
    if (source.type === "held" && typeof source.index === "number") {
      return [{ mime: source.media_type, seq, index: source.index }];
    }
    return [];
  });
}

/// A file sent with a message, when it is a picture.
export function sentPicture(file: ChatFile, seq: number): ChatPicture | null {
  if (file.data) return { mime: file.mime, data: file.data };
  if (typeof file.held === "number") return { mime: file.mime, seq, index: file.held };
  return null;
}

/// Where a held picture's bytes come from: the pane's channel to the host.
/// `known` answers at once with what has already been fetched, so a card
/// opened again draws without a blank in between. It answers `null` for a
/// picture the host said it does not have, and `undefined` for one not asked
/// about yet.
export type PictureSource = {
  known: (seq: number, index: number) => Picture | null | undefined;
  fetch: (seq: number, index: number) => Promise<Picture | null>;
};

/// Unset where there is no host to ask, as in a conversation read back from
/// this machine's history, and there a held picture is not drawn.
export const HeldPictures = createContext<PictureSource | null>(null);

export function ChatImage({ picture, className, alt = "" }: { picture: ChatPicture; className?: string; alt?: string }) {
  const source = useContext(HeldPictures);
  const held = "data" in picture ? null : picture;
  const [got, setGot] = useState(() => (held ? source?.known(held.seq, held.index) : undefined));
  const seq = held?.seq;
  const index = held?.index;
  useEffect(() => {
    if (seq === undefined || index === undefined || !source) return;
    let live = true;
    const had = source.known(seq, index);
    if (had !== undefined) {
      setGot(had);
      return;
    }
    source.fetch(seq, index).then(
      (p) => live && setGot(p),
      () => live && setGot(null),
    );
    return () => {
      live = false;
    };
  }, [source, seq, index]);

  const data = "data" in picture ? picture.data : got?.data;
  if (data) return <Zoomable className={className} src={`data:${got?.mime ?? picture.mime};base64,${data}`} alt={alt} />;
  // Asked for and not there, or nowhere to ask: as a picture whose bytes were
  // never kept has always been drawn, which is not at all.
  if (!held || !source || got === null) return null;
  return <span className={`${className ?? ""} chat-image-waiting`} role="img" aria-label="Loading picture" />;
}
