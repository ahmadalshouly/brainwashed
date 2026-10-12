import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  HostReplyError,
  type ChatMessage,
  type ChatModel,
  type ChatOptions,
  type GeneratedMedia,
  type InstalledModel,
  type MediaEvent,
  type MediaModel,
  type MediaRequest,
  type ToolCall,
} from "@brainwashed/api";
import type { ShownCall } from "../chatstore";
import { Markdown } from "../markdown";
import { Icon, type IconName } from "../icons";
import {
  ACCEPT,
  MAX_ATTACHMENTS,
  dataUrl,
  extension,
  prepare,
  type Draft,
} from "../attachments";
import {
  STYLES,
  deleteConversation,
  loadConversations,
  loadMediaPrefs,
  loadPrefs,
  newId,
  pruneConversations,
  saveConversation,
  saveMediaPrefs,
  savePrefs,
  sizeFor,
  sortChats,
  titleFor,
  toMarkdown,
  toOptions,
  type ChatPrefs,
  type Conversation,
  type MediaPrefs,
  type Shape,
  type Turn,
} from "../chatstore";
import { ACCENTS, MONO, swatch, type Look } from "../storage";
import { errorText, stateText, useHost, useLoad } from "../ui";

const MODEL_KEY = "brainwashed.model";

function loadModelChoice(): string {
  try {
    return localStorage.getItem(MODEL_KEY) ?? "local";
  } catch {
    return "local";
  }
}

function saveModelChoice(id: string) {
  try {
    localStorage.setItem(MODEL_KEY, id);
  } catch {
    // Remembered for this visit only.
  }
}

const SUGGESTIONS: { icon: IconName; title: string; prompt: string }[] = [
  {
    icon: "lightbulb",
    title: "Explain something",
    prompt: "Explain how a VPN works, simply",
  },
  {
    icon: "mail",
    title: "Write an email",
    prompt: "Write a polite email asking for a deadline extension",
  },
  {
    icon: "code",
    title: "Help with code",
    prompt:
      "Write a Python function that removes duplicates from a list, keeping order",
  },
  {
    icon: "sparkle",
    title: "Plan my week",
    prompt: "Plan my meals for this week",
  },
];

const PICTURE_IDEAS: { icon: IconName; title: string; prompt: string }[] = [
  {
    icon: "image",
    title: "A cozy scene",
    prompt:
      "A cozy reading nook by a rainy window at dusk, warm lamp light, a sleeping cat, soft film photo",
  },
  {
    icon: "sparkle",
    title: "A poster",
    prompt:
      'A retro travel poster of a mountain lake at sunrise with the words "VISIT HOME" in bold letters',
  },
  {
    icon: "palette",
    title: "An illustration",
    prompt:
      "A watercolor illustration of a fox reading a map in an autumn forest, children's book style",
  },
];

const VIDEO_IDEAS: { icon: IconName; title: string; prompt: string }[] = [
  {
    icon: "video",
    title: "Ocean waves",
    prompt:
      "Slow waves rolling onto a sandy beach at golden hour, gentle camera pan, cinematic",
  },
  {
    icon: "sparkle",
    title: "City lights",
    prompt:
      "A rainy neon-lit city street at night, people with umbrellas walking past, reflections on the wet road",
  },
  {
    icon: "palette",
    title: "A little animation",
    prompt:
      "A paper boat drifting down a stream past flowers, stop-motion style, soft daylight",
  },
];

function greeting(): string {
  const h = new Date().getHours();
  return h < 5
    ? "Up late?"
    : h < 12
      ? "Good morning"
      : h < 18
        ? "Good afternoon"
        : "Good evening";
}

/** The question when the model asks the person to pick an option. */
function askOf(c: ToolCall): { question: string; options: string[] } | null {
  if (c.name !== "ask_user" || !c.arguments || typeof c.arguments !== "object")
    return null;
  const a = c.arguments as { question?: unknown; options?: unknown };
  if (typeof a.question !== "string") return null;
  const options = Array.isArray(a.options)
    ? a.options.filter((o): o is string => typeof o === "string" && !!o.trim())
    : [];
  return { question: a.question, options };
}

/**
 * An answer's text split where it called tools, with the calls in between.
 * Questions for the person (`ask_user`) are shown after the answer instead.
 */
function answerPieces(t: Turn): (string | ShownCall)[] {
  const calls = (t.toolCalls ?? [])
    .filter((c) => !askOf(c))
    .map((c) => ({
      ...c,
      at: Math.min(c.at ?? t.content.length, t.content.length),
    }))
    .sort((a, b) => a.at - b.at);
  const out: (string | ShownCall)[] = [];
  let from = 0;
  for (const c of calls) {
    const text = t.content.slice(from, c.at);
    if (text.trim()) out.push(text);
    out.push(c);
    from = c.at;
  }
  const rest = t.content.slice(from);
  if (rest.trim()) out.push(rest);
  return out;
}

/** Whether a reply being written has nothing new to show yet. */
function waiting(t: Turn): boolean {
  const pieces = answerPieces(t);
  const last = pieces[pieces.length - 1];
  if (last === undefined) return !t.reasoning;
  // Back from a tool, before the model says more.
  return typeof last !== "string" && !!last.result;
}

/** A tool the model used: what it asked and what came back. */
function ToolCard({ call: c, running }: { call: ShownCall; running: boolean }) {
  const r = c.result;
  const name = c.name ? <code>{c.name}</code> : "a tool";
  return (
    <details className={`tool-call ${r?.isError ? "failed" : ""}`}>
      <summary>
        <Icon name="wrench" size={14} />
        {r ? (
          <span>
            {r.isError ? "Couldn't use " : "Used "}
            {name}
            {r.server && <span className="tool-server"> · {r.server}</span>}
          </span>
        ) : running ? (
          <span className="shimmer">Using {c.name || "a tool"}…</span>
        ) : (
          <span>Tried to use {name}</span>
        )}
        <Icon name="chevronDown" size={14} />
      </summary>
      <div className="tool-call-body">
        {!r && !running && (
          <p>
            BrainWashed doesn't have this tool, so nothing ran. Admins can add
            tools on the Tools page.
          </p>
        )}
        {c.raw !== undefined ||
        (c.arguments &&
          typeof c.arguments === "object" &&
          Object.keys(c.arguments).length) ? (
          <>
            <p className="tool-label">Asked with</p>
            <pre>{c.raw ?? JSON.stringify(c.arguments, null, 2)}</pre>
          </>
        ) : null}
        {r && (
          <>
            <p className="tool-label">
              {r.isError ? "What went wrong" : "Got back"}
            </p>
            <pre>{r.content || "(nothing)"}</pre>
          </>
        )}
      </div>
    </details>
  );
}

/** A turn's text with any question it asked, so the model sees what was asked. */
function fullText(t: Turn): string {
  const asks = (t.toolCalls ?? []).map(askOf).filter((a) => a !== null);
  return [
    t.content,
    ...asks.map((a) =>
      [a.question, ...a.options.map((o) => `- ${o}`)].join("\n"),
    ),
  ]
    .filter(Boolean)
    .join("\n\n");
}

/**
 * Messages as the model gets them: no errors, no UI-only fields. Tools that
 * ran go first as calls and their results, so the model remembers them.
 */
function wire(turns: Turn[], instructions?: string): ChatMessage[] {
  const out: ChatMessage[] = [];
  if (instructions?.trim())
    out.push({ role: "system", content: instructions.trim() });
  for (const t of turns) {
    const ran = (t.toolCalls ?? []).filter((c) => c.result);
    if (t.role === "assistant" && ran.length) {
      out.push({
        role: "assistant",
        content: "",
        toolCalls: ran.map(({ id, name, arguments: args }) => ({
          id,
          name,
          arguments: args,
        })),
      });
      for (const c of ran)
        out.push({
          role: "tool",
          toolCallId: c.id,
          content: c.result!.isError
            ? `Error: ${c.result!.content}`
            : c.result!.content,
        });
    }
    if (t.media) {
      // Chat models can't see what was made; tell them what it was.
      out.push({
        role: "assistant",
        content: `(Made a ${t.media.kind === "video" ? "video" : "picture"} with ${t.media.modelName}: "${t.media.prompt}")`,
      });
      continue;
    }
    const content = fullText(t);
    if (t.error && !content) continue;
    if (!content && !t.attachments?.length) continue;
    out.push(
      t.attachments?.length
        ? { role: t.role, content, attachments: t.attachments }
        : { role: t.role, content },
    );
  }
  return out;
}

function groupOf(c: Conversation): string {
  if (c.pinned) return "Pinned";
  const day = 24 * 3600 * 1000;
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const t = c.updatedAt;
  if (t >= start.getTime()) return "Today";
  if (t >= start.getTime() - day) return "Yesterday";
  if (t >= start.getTime() - 7 * day) return "Previous 7 days";
  if (t >= start.getTime() - 30 * day) return "Previous 30 days";
  return "Older";
}

function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "text/markdown" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function ChatPage({
  look,
  setLook,
  onForget,
  onManage,
}: {
  look: Look;
  setLook: (l: Look) => void;
  onForget: () => void;
  onManage: (page?: string) => void;
}) {
  const { remote, paired, role, state, refreshState, fail } = useHost();
  const admin = role === "admin";

  // ----- models -----
  const chatModels = useLoad<ChatModel[]>(
    () =>
      remote.chatModels().catch(async (): Promise<ChatModel[]> => {
        // Hosts before cloud providers only have the local model.
        const s = await remote.state();
        return s.state === "ready"
          ? [
              {
                id: "local",
                name: s.model,
                provider: null,
                vision: false,
                cloud: false,
              },
            ]
          : [];
      }),
    [remote, state?.state, state && "model" in state ? state.model : ""],
    20000,
  );
  // The admin's chat defaults: whatever a chat leaves unset.
  const hostDefaults = useLoad<ChatOptions>(
    () => remote.chatDefaults().catch(() => ({})),
    [remote],
    60000,
  );
  const defaults = hostDefaults.value ?? {};
  const installed = useLoad<InstalledModel[]>(
    () => (admin ? remote.models() : Promise.resolve([])),
    [remote, admin, state?.state],
    30000,
  );
  // Picture and video models. Hosts before them answer with an error.
  const mediaList = useLoad<MediaModel[]>(
    () => remote.mediaModels().catch(() => []),
    [remote],
    30000,
  );
  const mediaModels = mediaList.value ?? [];
  const [modelId, setModelId] = useState(loadModelChoice);
  const models = chatModels.value ?? [];
  const media = modelId.startsWith("media:")
    ? (mediaModels.find((m) => `media:${m.id}` === modelId) ?? null)
    : null;
  const model = media
    ? null
    : (models.find((m) => m.id === modelId) ?? models[0] ?? null);
  const ready = !!model || !!media;
  const loading =
    state?.state === "loading" || state?.state === "installingRuntime";
  const pickModel = (id: string) => {
    setModelId(id);
    saveModelChoice(id);
  };

  async function loadLocal(id: string) {
    try {
      await remote.loadModel(id);
      pickModel("local");
      refreshState();
    } catch (e) {
      fail(e);
    }
  }

  // ----- conversations -----
  const [chats, setChats] = useState<Conversation[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [instructions, setInstructions] = useState("");
  const [query, setQuery] = useState("");
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    loadConversations()
      .then(pruneConversations)
      .then((all) => {
        setChats(all);
        setLoaded(true);
      });
  }, []);

  // ----- composing -----
  const [input, setInput] = useState("");
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<{
    index: number;
    text: string;
  } | null>(null);
  const [prefs, setPrefsState] = useState<ChatPrefs>(loadPrefs);
  const setPrefs = (p: ChatPrefs) => {
    setPrefsState(p);
    savePrefs(p);
  };
  const [mediaPrefs, setMediaPrefsState] = useState<MediaPrefs>(loadMediaPrefs);
  const setMediaPrefs = (p: MediaPrefs) => {
    setMediaPrefsState(p);
    saveMediaPrefs(p);
  };
  const [zoom, setZoom] = useState<GeneratedMedia | null>(null);

  // ----- layout -----
  const [navOpen, setNavOpen] = useState(false);
  const [tuneOpen, setTuneOpen] = useState(false);
  const [menu, setMenu] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [atBottom, setAtBottom] = useState(true);

  const abort = useRef<AbortController | null>(null);
  const replyId = useRef<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLTextAreaElement>(null);
  const files = useRef<HTMLInputElement>(null);
  const search = useRef<HTMLInputElement>(null);

  const chatsRef = useRef(chats);
  chatsRef.current = chats;

  /** Saves a conversation after a reply, keeping its title, pin and instructions. */
  async function persist(
    id: string,
    nextTurns: Turn[],
    patch: Partial<Conversation> = {},
  ) {
    if (!nextTurns.length) return;
    const prev = chatsRef.current.find((c) => c.id === id);
    const saved: Conversation = {
      id,
      title:
        prev?.title && prev.title !== "New chat"
          ? prev.title
          : titleFor(nextTurns),
      turns: nextTurns,
      updatedAt: Date.now(),
      pinned: prev?.pinned,
      instructions: prev?.instructions,
      ...patch,
    };
    setChats((all) => sortChats([saved, ...all.filter((c) => c.id !== id)]));
    await saveConversation(saved);
  }

  // Tells the computer to stop writing too; closing the stream alone may not
  // reach it through a tunnel.
  function stop() {
    if (replyId.current) remote.chatStop(replyId.current).catch(() => {});
    abort.current?.abort();
  }

  function open(c: Conversation | null) {
    if (busy) stop();
    setActiveId(c?.id ?? null);
    setTurns(c?.turns ?? []);
    setInstructions(c?.instructions ?? "");
    setEditing(null);
    setNavOpen(false);
    setMenu(null);
    setTimeout(() => box.current?.focus(), 0);
  }

  async function remove(id: string) {
    setChats((all) => all.filter((c) => c.id !== id));
    await deleteConversation(id);
    if (id === activeId) open(null);
  }

  function updateChat(id: string, patch: Partial<Conversation>) {
    const c = chatsRef.current.find((x) => x.id === id);
    if (!c) return;
    const next = { ...c, ...patch };
    setChats((all) => sortChats(all.map((x) => (x.id === id ? next : x))));
    saveConversation(next);
  }

  // ----- sending -----
  async function ask(history: Turn[], chatId: string) {
    const placeholder: Turn = { role: "assistant", content: "" };
    setTurns([...history, placeholder]);
    setBusy(true);
    const controller = new AbortController();
    abort.current = controller;
    replyId.current = `web-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
    let thinkingStarted: number | null = null;
    let current: Turn = placeholder;
    const update = (f: (t: Turn) => Turn) => {
      current = f(current);
      const snapshot = current;
      setTurns((all) => [...all.slice(0, -1), snapshot]);
    };
    try {
      await remote.chat(
        wire(history, instructions),
        (e) =>
          update((t) => {
            switch (e.kind) {
              case "skills":
                return { ...t, skills: e.names };
              case "reasoning":
                thinkingStarted ??= Date.now();
                return { ...t, reasoning: (t.reasoning ?? "") + e.text };
              case "content":
                if (thinkingStarted && t.thoughtFor === undefined)
                  return {
                    ...t,
                    content: t.content + e.text,
                    thoughtFor: Math.max(
                      1,
                      Math.round((Date.now() - thinkingStarted) / 1000),
                    ),
                  };
                return { ...t, content: t.content + e.text };
              case "tool_call": {
                const { kind: _, ...call } = e;
                return {
                  ...t,
                  toolCalls: [
                    ...(t.toolCalls ?? []),
                    { ...call, at: t.content.length },
                  ],
                };
              }
              case "tool_result": {
                const { kind: _, ...result } = e;
                return {
                  ...t,
                  toolCalls: (t.toolCalls ?? []).map((c) =>
                    c.id === result.id ? { ...c, result } : c,
                  ),
                };
              }
              case "stats": {
                const { kind: _, ...stats } = e;
                return { ...t, stats };
              }
              default:
                return t;
            }
          }),
        controller.signal,
        {
          options: toOptions(prefs),
          model: model && model.id !== "local" ? model.id : undefined,
          replyId: replyId.current,
        },
      );
    } catch (e) {
      if (controller.signal.aborted) {
        update((t) => t);
      } else if (e instanceof HostReplyError && e.status === 401) {
        fail(e);
      } else {
        update((t) => ({ ...t, error: errorText(e) }));
      }
    } finally {
      if (thinkingStarted)
        update((t) =>
          t.thoughtFor === undefined
            ? {
                ...t,
                thoughtFor: Math.max(
                  1,
                  Math.round((Date.now() - thinkingStarted!) / 1000),
                ),
              }
            : t,
        );
      abort.current = null;
      replyId.current = null;
      setBusy(false);
      const finished = [...history, current];
      persist(chatId, finished, { instructions: instructions || undefined });
    }
  }

  /** Makes a picture or video from the last message, with the picked model. */
  async function make(history: Turn[], chatId: string, with_: MediaModel) {
    const user = history[history.length - 1];
    const picture = user?.attachments?.find((a) => a.type === "image");
    const d = with_.defaults;
    const { width, height } = sizeFor(mediaPrefs.shape, d.width, d.height);
    const video = with_.kind === "video";
    const fps = d.fps ?? 16;
    const request: MediaRequest = {
      model: with_.id,
      prompt: user?.content.trim() || "A picture like this one",
      width,
      height,
      ...(mediaPrefs.steps ? { steps: mediaPrefs.steps } : {}),
      ...(mediaPrefs.cfgScale !== undefined
        ? { cfgScale: mediaPrefs.cfgScale }
        : {}),
      ...(mediaPrefs.seed !== undefined ? { seed: mediaPrefs.seed } : {}),
      ...(mediaPrefs.negativePrompt?.trim()
        ? { negativePrompt: mediaPrefs.negativePrompt.trim() }
        : {}),
      ...(video && mediaPrefs.seconds
        ? { frames: Math.round(mediaPrefs.seconds * fps) + 1, fps }
        : {}),
      ...(picture && picture.type === "image"
        ? {
            image: picture.data,
            ...(mediaPrefs.strength !== undefined
              ? { strength: mediaPrefs.strength }
              : {}),
          }
        : {}),
    };
    const placeholder: Turn = {
      role: "assistant",
      content: "",
      making: {
        event: { kind: "progress", stage: "loading", step: 0, steps: 0 },
        kind: with_.kind,
        width,
        height,
        startedAt: Date.now(),
      },
    };
    let current = placeholder;
    const update = (f: (t: Turn) => Turn) => {
      current = f(current);
      const snapshot = current;
      setTurns((all) => [...all.slice(0, -1), snapshot]);
    };
    setTurns([...history, placeholder]);
    setBusy(true);
    const controller = new AbortController();
    abort.current = controller;
    replyId.current = `web-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
    try {
      const result = await remote.generate(
        request,
        (e: MediaEvent) =>
          update((t) =>
            t.making ? { ...t, making: { ...t.making, event: e } } : t,
          ),
        controller.signal,
        { replyId: replyId.current },
      );
      update((t) => ({ ...t, making: undefined, media: result }));
    } catch (e) {
      if (e instanceof HostReplyError && e.status === 401) fail(e);
      update((t) => ({
        ...t,
        making: undefined,
        error: controller.signal.aborted ? "Stopped." : errorText(e),
      }));
    } finally {
      abort.current = null;
      replyId.current = null;
      setBusy(false);
      persist(chatId, [...history, current], {
        instructions: instructions || undefined,
      });
    }
  }

  /** Answers with the chat model, or makes something with a picture model. */
  function reply(history: Turn[], chatId: string) {
    if (media) make(history, chatId, media);
    else ask(history, chatId);
  }

  const reading = drafts.some((d) => d.status === "reading");
  const ready_drafts = drafts.filter(
    (d) => d.status === "ready" && d.attachment,
  );
  const hasPictures = ready_drafts.some((d) => d.kind === "image");
  const blind = hasPictures && model && !model.vision;
  /** Why the attachments don't suit the picked picture or video model. */
  const mediaProblem = !media
    ? null
    : ready_drafts.some((d) => d.kind === "file")
      ? `${media.name} only takes a picture to start from, not documents.`
      : ready_drafts.length > 1
        ? "Attach one picture to start from."
        : hasPictures && media.kind === "video" && !media.pictureInput
          ? `${media.name} makes videos from a description only. Remove the picture, or pick a video model that starts from one.`
          : null;

  function send(text = input) {
    text = text.trim();
    if (
      (!text && !ready_drafts.length) ||
      busy ||
      !ready ||
      reading ||
      blind ||
      mediaProblem
    )
      return;
    const id = activeId ?? newId();
    if (!activeId) setActiveId(id);
    const user: Turn = {
      role: "user",
      content: text,
      ...(ready_drafts.length
        ? { attachments: ready_drafts.map((d) => d.attachment!) }
        : {}),
    };
    setInput("");
    setDrafts([]);
    setEditing(null);
    reply([...turns.filter((t) => !(t.error && !t.content)), user], id);
  }

  function regenerate(index = turns.length - 1) {
    const history = turns
      .slice(0, index)
      .filter((t) => !(t.error && !t.content));
    while (history.length && history[history.length - 1].role === "assistant")
      history.pop();
    if (history.length && activeId) reply(history, activeId);
  }

  function resend(index: number, text: string) {
    if (!activeId || busy) return;
    const original = turns[index];
    const history = [
      ...turns.slice(0, index),
      { ...original, content: text.trim() },
    ];
    setEditing(null);
    reply(history, activeId);
  }

  function continueReply() {
    if (!activeId || busy) return;
    ask(
      [
        ...turns,
        { role: "user", content: "Continue exactly where you stopped." },
      ],
      activeId,
    );
  }

  // ----- attachments -----
  async function attach(list: FileList | File[]) {
    const incoming = Array.from(list).slice(
      0,
      Math.max(0, MAX_ATTACHMENTS - drafts.length),
    );
    if (!incoming.length) return;
    const added = incoming.map(
      (f) =>
        ({
          id: newId(),
          name: f.name || "Pasted picture",
          kind: f.type.startsWith("image/") ? "image" : "file",
          status: "reading",
        }) as Draft,
    );
    setDrafts((d) => [...d, ...added]);
    await Promise.all(
      incoming.map(async (f, i) => {
        const done = await prepare(f, remote, added[i].id);
        setDrafts((d) => d.map((x) => (x.id === done.id ? done : x)));
      }),
    );
  }

  // ----- effects -----
  useEffect(() => {
    const el = scroller.current;
    if (el && atBottom) el.scrollTop = el.scrollHeight;
  }, [turns, atBottom]);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, window.innerHeight * 0.4)}px`;
  }, [input]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.shiftKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        open(null);
      } else if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setNavOpen(true);
        setTimeout(() => search.current?.focus(), 0);
      } else if (e.key === "Escape") {
        if (zoom) setZoom(null);
        else if (menu) setMenu(null);
        else if (tuneOpen) setTuneOpen(false);
        else if (busy) stop();
      }
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  });

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest(".menu, [data-menu]"))
        setMenu(null);
    };
    addEventListener("mousedown", close);
    return () => removeEventListener("mousedown", close);
  }, [menu]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return chats;
    return chats.filter(
      (c) =>
        c.title.toLowerCase().includes(q) ||
        c.turns.some((t) => t.content.toLowerCase().includes(q)),
    );
  }, [chats, query]);
  const groups = useMemo(() => {
    const out: [string, Conversation[]][] = [];
    for (const c of filtered) {
      const g = groupOf(c);
      const last = out[out.length - 1];
      if (last && last[0] === g) last[1].push(c);
      else out.push([g, [c]]);
    }
    return out;
  }, [filtered]);

  const active = chats.find((c) => c.id === activeId);
  const last = turns[turns.length - 1];
  const empty = turns.length === 0;
  const cloud = model?.cloud;
  const placeholder = !ready
    ? loading
      ? "Starting the model…"
      : "No model is running yet"
    : media
      ? media.kind === "video"
        ? hasPictures
          ? "Describe how the picture should move"
          : "Describe a video to make"
        : hasPictures
          ? "Describe how to change the picture"
          : "Describe a picture to make"
      : model?.cloud
        ? `Message ${model.name}`
        : `Ask ${paired.hostName}`;

  const composer = (
    <form
      className="composer glass"
      onSubmit={(e) => {
        e.preventDefault();
        send();
      }}
    >
      {drafts.length > 0 && (
        <div className="drafts">
          {drafts.map((d) => (
            <div
              key={d.id}
              className={`draft ${d.kind} ${d.status}`}
              title={d.error ?? d.name}
            >
              {d.kind === "image" && d.preview ? (
                <img src={d.preview} alt={d.name} />
              ) : (
                <>
                  <span className="ext">
                    {d.status === "reading" ? (
                      <span className="mini-spin" />
                    ) : (
                      extension(d.name)
                    )}
                  </span>
                  <span className="draft-text">
                    <strong>{d.name}</strong>
                    <small>
                      {d.status === "reading"
                        ? "Reading…"
                        : (d.error ?? d.detail)}
                    </small>
                  </span>
                </>
              )}
              <button
                type="button"
                className="draft-x"
                aria-label={`Remove ${d.name}`}
                onClick={() =>
                  setDrafts((all) => all.filter((x) => x.id !== d.id))
                }
              >
                <Icon name="close" size={12} />
              </button>
            </div>
          ))}
        </div>
      )}
      <textarea
        ref={box}
        value={input}
        placeholder={placeholder}
        disabled={!ready}
        rows={1}
        onChange={(e) => setInput(e.target.value)}
        onPaste={(e) => {
          const pasted = Array.from(e.clipboardData.files);
          if (pasted.length) {
            e.preventDefault();
            attach(pasted);
          }
        }}
        onKeyDown={(e) => {
          if (
            e.key === "Enter" &&
            !e.shiftKey &&
            !e.nativeEvent.isComposing &&
            !matchMedia("(pointer: coarse)").matches
          ) {
            e.preventDefault();
            send();
          }
        }}
      />
      {mediaProblem && <div className="composer-note warn">{mediaProblem}</div>}
      {blind && (
        <div className="composer-note warn">
          {model?.name} can't see pictures.{" "}
          {models.some((m) => m.vision)
            ? "Pick a model that can from the menu at the top."
            : admin
              ? "Download Gemma 3 4B or Qwen2.5 VL 3B in Models."
              : "Ask an admin for a model that can."}
        </div>
      )}
      <div className="composer-row">
        <button
          type="button"
          className="round"
          aria-label={
            media
              ? "Attach a picture to start from"
              : "Attach pictures or files"
          }
          title={
            media
              ? "Attach a picture to start from"
              : "Attach pictures, PDFs or files"
          }
          disabled={!ready || drafts.length >= MAX_ATTACHMENTS}
          onClick={() => files.current?.click()}
        >
          <Icon name="plus" size={20} />
        </button>
        {media ? (
          <ShapeChip
            shape={mediaPrefs.shape}
            onChange={(shape) => setMediaPrefs({ ...mediaPrefs, shape })}
          />
        ) : (
          <button
            type="button"
            className={`chip ${(prefs.reasoning ?? defaults.reasoning) ? "on" : ""}`}
            title="Whether reasoning models think before answering"
            onClick={() =>
              setPrefs({
                ...prefs,
                reasoning:
                  prefs.reasoning === undefined
                    ? false
                    : prefs.reasoning === false
                      ? true
                      : undefined,
              })
            }
          >
            <Icon name="brain" size={16} />
            {(prefs.reasoning ?? defaults.reasoning) === undefined
              ? "Think: auto"
              : (prefs.reasoning ?? defaults.reasoning)
                ? "Think: on"
                : "Think: off"}
          </button>
        )}
        <button
          type="button"
          className={`chip ${tuneOpen ? "on" : ""}`}
          onClick={() => setTuneOpen(!tuneOpen)}
          title="Model settings"
        >
          <Icon name="sliders" size={16} />
          {media
            ? "Settings"
            : prefs.style && prefs.style !== "custom"
              ? STYLES[prefs.style].label
              : prefs.temperature !== undefined
                ? `Temp ${prefs.temperature}`
                : "Settings"}
        </button>
        <span className="grow" />
        {busy ? (
          <button
            type="button"
            className="round send stop"
            aria-label="Stop"
            onClick={stop}
          >
            <Icon name="stop" size={16} />
          </button>
        ) : (
          <button
            className="round send"
            type="submit"
            aria-label="Send"
            disabled={
              !ready ||
              reading ||
              !!blind ||
              !!mediaProblem ||
              (!input.trim() && !ready_drafts.length)
            }
          >
            <Icon name="send" size={18} />
          </button>
        )}
      </div>
      <input
        ref={files}
        type="file"
        multiple
        accept={ACCEPT}
        hidden
        onChange={(e) => {
          if (e.target.files) attach(e.target.files);
          e.target.value = "";
        }}
      />
    </form>
  );

  return (
    <div
      className={`chat-app ${navOpen ? "nav-open" : ""} ${tuneOpen ? "tune-open" : ""}`}
      onDragOver={(e) => {
        if (ready && e.dataTransfer.types.includes("Files")) {
          e.preventDefault();
          setDragging(true);
        }
      }}
      onDragLeave={(e) => {
        if (
          e.currentTarget === e.target ||
          !(
            e.relatedTarget && e.currentTarget.contains(e.relatedTarget as Node)
          )
        )
          setDragging(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        setDragging(false);
        if (ready && e.dataTransfer.files.length) attach(e.dataTransfer.files);
      }}
    >
      <div className="aurora" aria-hidden>
        <i />
        <i />
        <i />
      </div>

      {/* ----- conversations ----- */}
      <aside className="chat-nav glass">
        <div className="nav-top">
          <label className="search">
            <Icon name="search" size={16} />
            <input
              ref={search}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search chats"
              aria-label="Search chats"
            />
          </label>
          <button
            className="icon-btn"
            aria-label="New chat"
            title="New chat (Ctrl+Shift+O)"
            onClick={() => open(null)}
          >
            <Icon name="newChat" />
          </button>
        </div>
        <div className="brand-row">
          <span className="orb small" />
          <strong>BrainWashed</strong>
        </div>
        <div className="convos">
          {loaded && chats.length === 0 && (
            <p className="hint">
              Your chats appear here. They're kept only in this browser.
            </p>
          )}
          {loaded && chats.length > 0 && filtered.length === 0 && (
            <p className="hint">No chats match.</p>
          )}
          {groups.map(([label, list]) => (
            <section key={label}>
              <h4>{label}</h4>
              {list.map((c) => (
                <div
                  key={c.id}
                  className={`convo ${c.id === activeId ? "active" : ""}`}
                >
                  <button
                    className="convo-open"
                    onClick={() => open(c)}
                    title={c.title}
                  >
                    {c.pinned && <Icon name="pin" size={13} />}
                    <span>{c.title}</span>
                  </button>
                  <button
                    className="convo-more"
                    data-menu
                    aria-label="Chat options"
                    onClick={() => setMenu(menu === c.id ? null : c.id)}
                  >
                    <Icon name="more" />
                  </button>
                  {menu === c.id && (
                    <div className="menu">
                      <button
                        onClick={() => {
                          const name = prompt("Rename chat", c.title);
                          if (name?.trim())
                            updateChat(c.id, { title: name.trim() });
                          setMenu(null);
                        }}
                      >
                        <Icon name="edit" size={16} /> Rename
                      </button>
                      <button
                        onClick={() => {
                          updateChat(c.id, { pinned: !c.pinned });
                          setMenu(null);
                        }}
                      >
                        <Icon name="pin" size={16} />{" "}
                        {c.pinned ? "Unpin" : "Pin"}
                      </button>
                      <button
                        onClick={() => {
                          download(
                            `${c.title.replace(/[^\w -]+/g, "").trim() || "chat"}.md`,
                            toMarkdown(c),
                          );
                          setMenu(null);
                        }}
                      >
                        <Icon name="download" size={16} /> Save as Markdown
                      </button>
                      <button
                        className="danger"
                        onClick={() => {
                          setMenu(null);
                          if (confirm(`Delete "${c.title}"?`)) remove(c.id);
                        }}
                      >
                        <Icon name="trash" size={16} /> Delete
                      </button>
                    </div>
                  )}
                </div>
              ))}
            </section>
          ))}
        </div>
        <div className="nav-foot">
          <div className="host-card">
            <span className="host-icon">
              <Icon name="dashboard" size={18} />
            </span>
            <span className="host-text">
              <strong>{paired.hostName}</strong>
              <small className={state?.state === "ready" ? "ok" : ""}>
                {stateText(
                  state,
                  (id) => installed.value?.find((m) => m.id === id)?.name ?? id,
                )}
              </small>
            </span>
            <button
              className="icon-btn"
              data-menu
              aria-label="Preferences"
              onClick={() => setMenu(menu === "prefs" ? null : "prefs")}
            >
              <Icon name="settings" />
            </button>
          </div>
          {admin && (
            <button className="manage" onClick={() => onManage()}>
              <Icon name="dashboard" size={16} /> Manage this computer
            </button>
          )}
          {menu === "prefs" && (
            <div className="menu up prefs-menu">
              <label>
                Theme
                <select
                  value={look.theme}
                  onChange={(e) =>
                    setLook({ ...look, theme: e.target.value as Look["theme"] })
                  }
                >
                  <option value="auto">Match device</option>
                  <option value="light">Light</option>
                  <option value="dark">Dark</option>
                </select>
              </label>
              <div
                className="accents"
                role="radiogroup"
                aria-label="Accent color"
              >
                {ACCENTS.map((c) => (
                  <button
                    key={c}
                    role="radio"
                    aria-checked={look.accent === c}
                    aria-label={c === MONO ? "Black and white" : c}
                    className={look.accent === c ? "swatch on" : "swatch"}
                    style={{ background: swatch(c) }}
                    onClick={() => setLook({ ...look, accent: c })}
                  />
                ))}
              </div>
              <span className="role-line">
                {admin ? "This browser is an admin." : "This browser can chat."}
              </span>
              <button
                className="danger"
                onClick={() => {
                  if (
                    confirm(
                      `Disconnect this browser from ${paired.hostName}? Chats stay in this browser until you clear them.`,
                    )
                  )
                    onForget();
                }}
              >
                <Icon name="logout" size={16} /> Disconnect this browser
              </button>
            </div>
          )}
        </div>
      </aside>
      <div className="scrim" onClick={() => setNavOpen(false)} />

      {/* ----- conversation ----- */}
      <main className="chat-stage">
        <header className="stage-top">
          <button
            className="icon-btn glass only-narrow"
            aria-label="Chats"
            onClick={() => setNavOpen(true)}
          >
            <Icon name="menu" />
          </button>
          <div className="model-pill-wrap">
            <button
              className="model-pill glass"
              data-menu
              onClick={() => setMenu(menu === "model" ? null : "model")}
              aria-haspopup="menu"
            >
              <span
                className={`dot ${ready ? (cloud ? "cloud" : "on") : loading ? "wait" : "off"}`}
              />
              {media && (
                <Icon
                  name={media.kind === "video" ? "video" : "image"}
                  size={16}
                />
              )}
              <span className="pill-text">
                <strong>
                  {media?.name ??
                    model?.name ??
                    (loading ? "Starting…" : "No model")}
                </strong>
                <small>
                  {media
                    ? `${media.kind === "video" ? "Videos" : "Pictures"} · ${paired.hostName}`
                    : model?.cloud
                      ? model.provider
                      : paired.hostName}
                </small>
              </span>
              <Icon name="chevronDown" size={16} />
            </button>
            {menu === "model" && (
              <div className="menu model-menu">
                {models.length === 0 && (
                  <p className="menu-note">
                    {loading ? stateText(state) : "Nothing is running yet."}
                  </p>
                )}
                {models.some((m) => !m.cloud) && <h5>On {paired.hostName}</h5>}
                {models
                  .filter((m) => !m.cloud)
                  .map((m) => (
                    <ModelRow
                      key={m.id}
                      m={m}
                      on={m.id === model?.id}
                      onPick={() => (pickModel(m.id), setMenu(null))}
                    />
                  ))}
                {admin &&
                  (installed.value ?? [])
                    .filter(
                      (m) =>
                        !(state?.state === "ready" && state.model === m.id),
                    )
                    .map((m) => (
                      <button
                        key={m.id}
                        className="model-row"
                        disabled={loading}
                        onClick={() => (loadLocal(m.id), setMenu(null))}
                      >
                        <span className="model-row-text">
                          <strong>{m.name}</strong>
                          <small>
                            {state?.state === "loading" && state.model === m.id
                              ? "Starting…"
                              : "Installed. Click to start it"}
                          </small>
                        </span>
                        {m.mmproj && <span className="tag">Vision</span>}
                      </button>
                    ))}
                {(["image", "video"] as const).map((kind) =>
                  mediaModels.some((m) => m.kind === kind) ? (
                    <div key={kind}>
                      <h5>
                        {kind === "image" ? "Make pictures" : "Make videos"}
                      </h5>
                      {mediaModels
                        .filter((m) => m.kind === kind)
                        .map((m) => (
                          <MediaRow
                            key={m.id}
                            m={m}
                            on={m.id === media?.id}
                            onPick={() => (
                              pickModel(`media:${m.id}`),
                              setMenu(null)
                            )}
                          />
                        ))}
                    </div>
                  ) : null,
                )}
                {models.some((m) => m.cloud) && <h5>Cloud</h5>}
                {models
                  .filter((m) => m.cloud)
                  .map((m) => (
                    <ModelRow
                      key={m.id}
                      m={m}
                      on={m.id === model?.id}
                      onPick={() => (pickModel(m.id), setMenu(null))}
                    />
                  ))}
                {admin && (
                  <div className="menu-actions">
                    <button onClick={() => onManage("models")}>
                      <Icon name="download" size={16} /> Get more models
                    </button>
                    {mediaModels.length === 0 && (
                      <button onClick={() => onManage("models")}>
                        <Icon name="image" size={16} /> Get picture and video
                        models
                      </button>
                    )}
                    <button onClick={() => onManage("providers")}>
                      <Icon name="globe" size={16} /> Connect a cloud provider
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
          <div className="top-right">
            <button
              className={`icon-btn glass ${tuneOpen ? "on" : ""}`}
              aria-label="Model settings"
              title="Model settings"
              onClick={() => setTuneOpen(!tuneOpen)}
            >
              <Icon name="sliders" />
            </button>
            <button
              className="icon-btn glass"
              aria-label="New chat"
              title="New chat"
              onClick={() => open(null)}
            >
              <Icon name="newChat" />
            </button>
          </div>
        </header>

        <div
          className={`stage-scroll ${empty ? "is-empty" : ""}`}
          ref={scroller}
          onScroll={(e) => {
            const el = e.currentTarget;
            setAtBottom(el.scrollHeight - el.scrollTop - el.clientHeight < 80);
          }}
        >
          {empty ? (
            <div className="welcome-hero">
              <span className="orb big" />
              <h1>
                {media
                  ? media.kind === "video"
                    ? "What should move?"
                    : "What should I make?"
                  : ready
                    ? greeting()
                    : loading
                      ? "Starting your model…"
                      : "Almost there"}
              </h1>
              <p className="lede">
                {media
                  ? `${media.name} makes ${media.kind === "video" ? "short videos" : "pictures"} on ${paired.hostName}, privately.${media.kind === "video" || media.pictureInput ? "" : " Attach a picture to change it."}${media.kind === "video" && media.pictureInput ? " Attach a picture to set it in motion." : ""}`
                  : ready
                    ? cloud
                      ? `Chatting with ${model!.name} from ${model!.provider}.`
                      : `What can I help with? Everything stays on ${paired.hostName}.`
                    : loading
                      ? stateText(state)
                      : admin
                        ? "Download a model to run on this computer, or connect a cloud provider."
                        : "An admin needs to start a model before you can chat."}
              </p>
              {!ready && !loading && admin && (
                <div className="hero-actions">
                  <button
                    className="primary"
                    onClick={() => onManage("models")}
                  >
                    <Icon name="download" size={16} /> Download a model
                  </button>
                  <button onClick={() => onManage("providers")}>
                    <Icon name="globe" size={16} /> Use a cloud provider
                  </button>
                </div>
              )}
              {ready && (
                <>
                  {composer}
                  <div className="cards">
                    {(media
                      ? media.kind === "video"
                        ? VIDEO_IDEAS
                        : PICTURE_IDEAS
                      : SUGGESTIONS
                    ).map((s) => (
                      <button
                        key={s.title}
                        className="card-suggest glass"
                        onClick={() => send(s.prompt)}
                      >
                        <Icon name={s.icon} />
                        <strong>{s.title}</strong>
                        <span>{s.prompt}</span>
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          ) : (
            <div className="thread">
              {active?.instructions && (
                <div className="instructions-note">
                  <Icon name="sparkle" size={14} /> Custom instructions are on
                  for this chat
                </div>
              )}
              {turns.map((t, i) =>
                t.role === "user" ? (
                  <div key={i} className="msg user">
                    {t.attachments && t.attachments.length > 0 && (
                      <div className="msg-files">
                        {t.attachments.map((a, j) =>
                          a.type === "image" ? (
                            <img key={j} src={dataUrl(a)} alt={a.name} />
                          ) : (
                            <span key={j} className="file-pill">
                              <span className="ext">{extension(a.name)}</span>
                              {a.name}
                            </span>
                          ),
                        )}
                      </div>
                    )}
                    {editing?.index === i ? (
                      <div className="edit-box glass">
                        <textarea
                          autoFocus
                          value={editing.text}
                          onChange={(e) =>
                            setEditing({ index: i, text: e.target.value })
                          }
                          rows={3}
                        />
                        <div className="edit-actions">
                          <button onClick={() => setEditing(null)}>
                            Cancel
                          </button>
                          <button
                            className="primary"
                            disabled={!editing.text.trim() || !ready}
                            onClick={() => resend(i, editing.text)}
                          >
                            Send
                          </button>
                        </div>
                      </div>
                    ) : (
                      t.content && <div className="bubble">{t.content}</div>
                    )}
                    {!busy && editing?.index !== i && (
                      <div className="msg-actions">
                        <CopyIcon text={t.content} />
                        <button
                          aria-label="Edit"
                          title="Edit and send again"
                          onClick={() =>
                            setEditing({ index: i, text: t.content })
                          }
                        >
                          <Icon name="edit" size={16} />
                        </button>
                      </div>
                    )}
                  </div>
                ) : (
                  <div key={i} className="msg assistant">
                    <span className="orb tiny" aria-hidden />
                    <div className="answer">
                      {t.skills && t.skills.length > 0 && (
                        <div className="skill-chips">
                          {t.skills.map((s) => (
                            <span key={s}>
                              <Icon name="sparkle" size={12} /> {s}
                            </span>
                          ))}
                        </div>
                      )}
                      {t.reasoning && (
                        <details
                          className="thought"
                          open={busy && i === turns.length - 1 && !t.content}
                        >
                          <summary>
                            {busy && i === turns.length - 1 && !t.content ? (
                              <span className="shimmer">Thinking…</span>
                            ) : (
                              <>Thought for {t.thoughtFor ?? 1}s</>
                            )}
                            <Icon name="chevronDown" size={14} />
                          </summary>
                          <div className="thought-body">{t.reasoning}</div>
                        </details>
                      )}
                      {t.making && <Making making={t.making} />}
                      {t.media && (
                        <MediaResult
                          m={t.media}
                          onZoom={() => setZoom(t.media!)}
                          onAgain={
                            i === turns.length - 1 && ready && !busy
                              ? () => regenerate()
                              : undefined
                          }
                          onStartFrom={
                            t.media.kind === "image" && !busy
                              ? () => {
                                  const m = t.media!;
                                  setDrafts([
                                    {
                                      id: newId(),
                                      name: "Made picture.png",
                                      kind: "image",
                                      status: "ready",
                                      preview: `data:${m.mime};base64,${m.data}`,
                                      attachment: {
                                        type: "image",
                                        name: "Made picture.png",
                                        mime: m.mime,
                                        data: m.data,
                                      },
                                    },
                                  ]);
                                  box.current?.focus();
                                }
                              : undefined
                          }
                        />
                      )}
                      {answerPieces(t).map((p, k) =>
                        typeof p === "string" ? (
                          <Markdown key={k} text={p} />
                        ) : (
                          <ToolCard
                            key={k}
                            call={p}
                            running={busy && i === turns.length - 1}
                          />
                        ),
                      )}
                      {busy &&
                      i === turns.length - 1 &&
                      !t.making &&
                      waiting(t) ? (
                        <span className="typing">
                          <i />
                          <i />
                          <i />
                        </span>
                      ) : null}
                      {t.toolCalls?.map((c, k) => {
                        const ask = askOf(c);
                        if (!ask) return null;
                        return (
                          <div key={k} className="ask">
                            <Markdown text={ask.question} />
                            {ask.options.length > 0 && (
                              <div className="ask-options">
                                {ask.options.map((o) => (
                                  <button
                                    key={o}
                                    disabled={
                                      i !== turns.length - 1 || !ready || busy
                                    }
                                    onClick={() => send(o)}
                                  >
                                    {o}
                                  </button>
                                ))}
                              </div>
                            )}
                          </div>
                        );
                      })}
                      {t.error && (
                        <div className="msg-error">
                          {t.error}
                          {i === turns.length - 1 && ready && !busy && (
                            <button onClick={() => regenerate()}>
                              <Icon name="refresh" size={14} /> Try again
                            </button>
                          )}
                        </div>
                      )}
                      {t.stats?.truncated &&
                        !busy &&
                        i === turns.length - 1 && (
                          <div className="msg-note">
                            The reply hit the length limit.
                            <button onClick={continueReply}>Continue</button>
                          </div>
                        )}
                      {!(busy && i === turns.length - 1) && fullText(t) && (
                        <div className="msg-actions">
                          <CopyIcon text={fullText(t)} />
                          {i === turns.length - 1 && ready && (
                            <button
                              aria-label="Regenerate"
                              title="Regenerate"
                              onClick={() => regenerate()}
                            >
                              <Icon name="refresh" size={16} />
                            </button>
                          )}
                          {t.stats && t.stats.tokens > 0 && (
                            <span
                              className="stats"
                              title={`${t.stats.promptTokens} tokens in, ${t.stats.tokens} out`}
                            >
                              {t.stats.tokens} tokens ·{" "}
                              {t.stats.tokensPerSecond.toFixed(1)}/s
                            </span>
                          )}
                        </div>
                      )}
                    </div>
                  </div>
                ),
              )}
              {last?.role === "user" && !busy && (
                <div className="msg-note">
                  No reply yet.
                  {ready && (
                    <button onClick={() => regenerate(turns.length)}>
                      Get a reply
                    </button>
                  )}
                </div>
              )}
            </div>
          )}
        </div>

        {!empty && (
          <div className="stage-bottom">
            {!atBottom && (
              <button
                className="jump glass"
                aria-label="Scroll to the latest message"
                onClick={() =>
                  scroller.current?.scrollTo({
                    top: scroller.current.scrollHeight,
                    behavior: "smooth",
                  })
                }
              >
                <Icon name="arrowDown" size={16} />
              </button>
            )}
            {composer}
          </div>
        )}
        <p className="fine-print">
          <Icon name={cloud ? "globe" : "lock"} size={12} />
          {media
            ? `Made privately on ${paired.hostName}. ${media.kind === "video" ? "Videos" : "Pictures"} can get details wrong.`
            : cloud
              ? `Messages in this chat go to ${model!.provider}. Answers can be wrong.`
              : `Runs privately on ${paired.hostName}. Answers can be wrong.`}
        </p>
        {dragging && (
          <div className="drop-zone">
            <div>
              <Icon name="paperclip" size={28} />
              <strong>Drop pictures, PDFs or files</strong>
            </div>
          </div>
        )}
      </main>

      {/* ----- model settings ----- */}
      <aside
        className="tune glass"
        aria-label="Model settings"
        aria-hidden={!tuneOpen}
      >
        {media ? (
          <MediaTunePanel
            model={media}
            prefs={mediaPrefs}
            setPrefs={setMediaPrefs}
            onClose={() => setTuneOpen(false)}
          />
        ) : (
          <TunePanel
            prefs={prefs}
            defaults={defaults}
            setPrefs={setPrefs}
            cloud={!!cloud}
            instructions={instructions}
            setInstructions={(v) => {
              setInstructions(v);
              if (activeId && chats.some((c) => c.id === activeId))
                updateChat(activeId, { instructions: v || undefined });
            }}
            onClose={() => setTuneOpen(false)}
          />
        )}
      </aside>
      {zoom && (
        <div
          className="lightbox"
          role="dialog"
          aria-label={zoom.prompt}
          onClick={() => setZoom(null)}
        >
          <img
            src={`data:${zoom.mime};base64,${zoom.data}`}
            alt={zoom.prompt}
          />
          <p>{zoom.prompt}</p>
        </div>
      )}
    </div>
  );
}

function ModelRow({
  m,
  on,
  onPick,
}: {
  m: ChatModel;
  on: boolean;
  onPick: () => void;
}) {
  return (
    <button
      className={`model-row ${on ? "on" : ""}`}
      onClick={onPick}
      role="menuitemradio"
      aria-checked={on}
    >
      <span className="model-row-text">
        <strong>{m.name}</strong>
        <small>
          {m.cloud
            ? `${m.provider} · leaves this computer`
            : "Running now · private"}
        </small>
      </span>
      {m.vision && !m.cloud && <span className="tag">Vision</span>}
      {on && <Icon name="check" size={16} />}
    </button>
  );
}

function CopyIcon({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      aria-label="Copy"
      title="Copy"
      onClick={() =>
        navigator.clipboard?.writeText(text).then(() => {
          setDone(true);
          setTimeout(() => setDone(false), 1400);
        })
      }
    >
      <Icon name={done ? "check" : "copy"} size={16} />
    </button>
  );
}

const LENGTHS = [undefined, 256, 512, 1024, 2048, 4096, 8192] as const;

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="field">
      <div className="field-head">
        <span>{label}</span>
        {hint && <small>{hint}</small>}
      </div>
      {children}
    </div>
  );
}

function NumberAuto({
  value,
  fallback,
  onChange,
  step,
  min,
  max,
  label,
}: {
  value: number | undefined;
  /** The host's default, shown when the chat doesn't set one. */
  fallback?: number;
  onChange: (v: number | undefined) => void;
  step: number;
  min: number;
  max: number;
  label: string;
}) {
  return (
    <label className="num">
      <span>{label}</span>
      <input
        type="number"
        inputMode="decimal"
        placeholder={fallback === undefined ? "Auto" : String(fallback)}
        step={step}
        min={min}
        max={max}
        value={value ?? ""}
        onChange={(e) => {
          const v = e.target.value === "" ? undefined : Number(e.target.value);
          onChange(
            v === undefined || Number.isNaN(v)
              ? undefined
              : Math.min(max, Math.max(min, v)),
          );
        }}
      />
    </label>
  );
}

function TunePanel({
  prefs,
  defaults,
  setPrefs,
  cloud,
  instructions,
  setInstructions,
  onClose,
}: {
  prefs: ChatPrefs;
  defaults: ChatOptions;
  setPrefs: (p: ChatPrefs) => void;
  cloud: boolean;
  instructions: string;
  setInstructions: (v: string) => void;
  onClose: () => void;
}) {
  const set = (patch: Partial<ChatPrefs>) => setPrefs({ ...prefs, ...patch });
  const lengthIndex = Math.max(
    0,
    LENGTHS.indexOf(prefs.maxTokens as (typeof LENGTHS)[number]),
  );
  return (
    <div className="tune-inner">
      <div className="tune-head">
        <h3>Model settings</h3>
        <button className="icon-btn" aria-label="Close" onClick={onClose}>
          <Icon name="close" />
        </button>
      </div>

      <Field
        label="Thinking"
        hint="Reasoning models like Qwen3 can think before they answer. Off is faster."
      >
        <div className="segmented">
          {(
            [
              [
                undefined,
                defaults.reasoning === undefined
                  ? "Default"
                  : `Default (${defaults.reasoning ? "on" : "off"})`,
              ],
              [true, "On"],
              [false, "Off"],
            ] as const
          ).map(([v, label]) => (
            <button
              key={label}
              className={prefs.reasoning === v ? "on" : ""}
              onClick={() => set({ reasoning: v })}
            >
              {label}
            </button>
          ))}
        </div>
      </Field>

      <Field
        label="Style"
        hint={
          prefs.style && prefs.style !== "custom"
            ? STYLES[prefs.style].hint
            : "How adventurous the wording is."
        }
      >
        <div className="segmented">
          <button
            className={
              !prefs.style && prefs.temperature === undefined ? "on" : ""
            }
            onClick={() => set({ style: undefined, temperature: undefined })}
          >
            Default
          </button>
          {(Object.keys(STYLES) as (keyof typeof STYLES)[]).map((k) => (
            <button
              key={k}
              className={prefs.style === k ? "on" : ""}
              onClick={() =>
                set({ style: k, temperature: STYLES[k].temperature })
              }
            >
              {STYLES[k].label}
            </button>
          ))}
        </div>
        <div className="slider-row">
          <span>Temperature</span>
          <input
            type="range"
            min={0}
            max={2}
            step={0.05}
            value={prefs.temperature ?? defaults.temperature ?? 0.8}
            onChange={(e) =>
              set({ temperature: Number(e.target.value), style: "custom" })
            }
            aria-label="Temperature"
          />
          <output>
            {prefs.temperature?.toFixed(2) ??
              (defaults.temperature !== undefined
                ? `${defaults.temperature.toFixed(2)} (default)`
                : "Auto")}
          </output>
        </div>
      </Field>

      <Field label="Reply length" hint="The longest a reply can be.">
        <div className="slider-row">
          <input
            type="range"
            min={0}
            max={LENGTHS.length - 1}
            step={1}
            value={lengthIndex}
            onChange={(e) =>
              set({ maxTokens: LENGTHS[Number(e.target.value)] })
            }
            aria-label="Reply length"
          />
          <output>
            {prefs.maxTokens
              ? `${prefs.maxTokens} tokens`
              : defaults.maxTokens
                ? `${defaults.maxTokens} tokens (default)`
                : "No limit"}
          </output>
        </div>
      </Field>

      <details className="tune-advanced">
        <summary>
          Advanced sampling <Icon name="chevronDown" size={14} />
        </summary>
        <div className="num-grid">
          <NumberAuto
            label="Top P"
            value={prefs.topP}
            fallback={defaults.topP}
            onChange={(v) => set({ topP: v })}
            step={0.05}
            min={0}
            max={1}
          />
          <NumberAuto
            label="Presence penalty"
            value={prefs.presencePenalty}
            fallback={defaults.presencePenalty}
            onChange={(v) => set({ presencePenalty: v })}
            step={0.1}
            min={-2}
            max={2}
          />
          <NumberAuto
            label="Top K"
            value={prefs.topK}
            fallback={defaults.topK}
            onChange={(v) =>
              set({ topK: v === undefined ? undefined : Math.round(v) })
            }
            step={1}
            min={0}
            max={1000}
          />
          <NumberAuto
            label="Min P"
            value={prefs.minP}
            fallback={defaults.minP}
            onChange={(v) => set({ minP: v })}
            step={0.01}
            min={0}
            max={1}
          />
          <NumberAuto
            label="Repeat penalty"
            value={prefs.repeatPenalty}
            fallback={defaults.repeatPenalty}
            onChange={(v) => set({ repeatPenalty: v })}
            step={0.05}
            min={0.5}
            max={2}
          />
          <NumberAuto
            label="Seed"
            value={prefs.seed}
            fallback={defaults.seed}
            onChange={(v) =>
              set({ seed: v === undefined ? undefined : Math.round(v) })
            }
            step={1}
            min={-1}
            max={2 ** 31}
          />
        </div>
        {cloud && (
          <p className="tune-note">
            Cloud models ignore Top K, Min P and Repeat penalty.
          </p>
        )}
      </details>

      <Field
        label="Instructions for this chat"
        hint="Added to the system prompt, for this conversation only."
      >
        <textarea
          className="instructions"
          rows={4}
          placeholder="For example: Answer in Arabic. Keep replies under 100 words."
          value={instructions}
          onChange={(e) => setInstructions(e.target.value)}
        />
      </Field>

      <div className="tune-foot">
        <button onClick={() => setPrefs({})}>Reset to defaults</button>
        <small>
          Saved in this browser. Admins set the defaults in Settings.
        </small>
      </div>
    </div>
  );
}

function MediaRow({
  m,
  on,
  onPick,
}: {
  m: MediaModel;
  on: boolean;
  onPick: () => void;
}) {
  return (
    <button
      className={`model-row ${on ? "on" : ""}`}
      onClick={onPick}
      role="menuitemradio"
      aria-checked={on}
    >
      <span className="model-row-text">
        <strong>{m.name}</strong>
        <small>
          {m.kind === "video"
            ? `Short videos · ${m.defaults.width}×${m.defaults.height}${m.pictureInput ? " · from a picture too" : ""}`
            : `Pictures · ${m.defaults.width}×${m.defaults.height}`}
        </small>
      </span>
      <span className="tag">{m.kind === "video" ? "Video" : "Picture"}</span>
      {on && <Icon name="check" size={16} />}
    </button>
  );
}

const SHAPES: { shape: Shape | undefined; label: string }[] = [
  { shape: undefined, label: "Model's size" },
  { shape: "square", label: "Square" },
  { shape: "portrait", label: "Portrait" },
  { shape: "landscape", label: "Landscape" },
];

/** Picks the shape of what's made, cycling through the options. */
function ShapeChip({
  shape,
  onChange,
}: {
  shape: Shape | undefined;
  onChange: (s: Shape | undefined) => void;
}) {
  const i = SHAPES.findIndex((s) => s.shape === shape);
  return (
    <button
      type="button"
      className={`chip ${shape ? "on" : ""}`}
      title="Shape of the picture or video"
      onClick={() => onChange(SHAPES[(i + 1) % SHAPES.length].shape)}
    >
      <span className={`shape-glyph ${shape ?? "auto"}`} aria-hidden />
      {SHAPES[Math.max(0, i)].label}
    </button>
  );
}

function makingText(e: MediaEvent, video: boolean): string {
  switch (e.kind) {
    case "queued":
      return "Waiting for another picture or video to finish…";
    case "installing":
      return e.total
        ? `Getting ready the first time: downloading stable-diffusion.cpp (${Math.round((e.done / e.total) * 100)}%)…`
        : "Getting ready the first time: downloading stable-diffusion.cpp…";
    case "progress":
      if (e.stage === "loading") return "Loading the model…";
      if (e.stage === "finishing")
        return video ? "Putting the frames together…" : "Finishing…";
      return `${video ? "Making the video" : "Drawing"}… step ${e.step} of ${e.steps}`;
  }
}

/** A picture or video on its way: its shape, shimmering, with progress. */
function Making({ making }: { making: NonNullable<Turn["making"]> }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  const e = making.event;
  const pct =
    e.kind === "progress" && e.steps > 0
      ? (e.step / e.steps) * 100
      : e.kind === "installing" && e.total
        ? (e.done / e.total) * 100
        : null;
  const seconds = Math.max(0, Math.round((now - making.startedAt) / 1000));
  return (
    <div className="making">
      <div
        className="making-canvas"
        style={{ aspectRatio: `${making.width} / ${making.height}` }}
      >
        <Icon name={making.kind === "video" ? "video" : "image"} size={28} />
      </div>
      <div className="making-text">
        <span className="shimmer">
          {makingText(e, making.kind === "video")}
        </span>
        <small>
          {seconds < 60
            ? `${seconds}s`
            : `${Math.floor(seconds / 60)}m ${seconds % 60}s`}
        </small>
      </div>
      <div className="making-bar" aria-hidden>
        <i
          className={pct === null ? "indeterminate" : ""}
          style={pct === null ? undefined : { width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

function saveMedia(m: GeneratedMedia) {
  const bytes = Uint8Array.from(atob(m.data), (c) => c.charCodeAt(0));
  const url = URL.createObjectURL(new Blob([bytes], { type: m.mime }));
  const a = document.createElement("a");
  a.href = url;
  const name =
    m.prompt
      .replace(/[^\w -]+/g, "")
      .trim()
      .split(/\s+/)
      .slice(0, 6)
      .join("-")
      .toLowerCase() || "brainwashed";
  a.download = `${name}.${m.mime === "image/webp" ? "webp" : "png"}`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function MediaResult({
  m,
  onZoom,
  onAgain,
  onStartFrom,
}: {
  m: GeneratedMedia;
  onZoom: () => void;
  onAgain?: () => void;
  onStartFrom?: () => void;
}) {
  const video = m.kind === "video";
  const length =
    video && m.frames && m.fps ? `${(m.frames / m.fps).toFixed(1)}s · ` : "";
  return (
    <figure className="media-result">
      <button
        className="media-frame"
        onClick={onZoom}
        aria-label="Show it bigger"
        style={{ aspectRatio: `${m.width} / ${m.height}` }}
      >
        <img src={`data:${m.mime};base64,${m.data}`} alt={m.prompt} />
        {video && (
          <span className="media-badge">
            <Icon name="video" size={12} /> Video
          </span>
        )}
      </button>
      <figcaption>
        {m.modelName} · {m.width}×{m.height} · {length}seed {m.seed} ·{" "}
        {m.seconds < 90
          ? `${Math.round(m.seconds)}s`
          : `${Math.round(m.seconds / 60)} min`}
      </figcaption>
      <div className="msg-actions">
        <button
          aria-label="Download"
          title={video ? "Download (animated WebP)" : "Download"}
          onClick={() => saveMedia(m)}
        >
          <Icon name="download" size={16} />
        </button>
        <CopyIcon text={m.prompt} />
        {onStartFrom && (
          <button
            aria-label="Start from this picture"
            title="Change this picture, or set it in motion with a video model"
            onClick={onStartFrom}
          >
            <Icon name="wand" size={16} />
          </button>
        )}
        {onAgain && (
          <button
            aria-label="Make another"
            title="Make another"
            onClick={onAgain}
          >
            <Icon name="refresh" size={16} />
          </button>
        )}
      </div>
    </figure>
  );
}

function MediaTunePanel({
  model,
  prefs,
  setPrefs,
  onClose,
}: {
  model: MediaModel;
  prefs: MediaPrefs;
  setPrefs: (p: MediaPrefs) => void;
  onClose: () => void;
}) {
  const set = (patch: Partial<MediaPrefs>) => setPrefs({ ...prefs, ...patch });
  const d = model.defaults;
  const video = model.kind === "video";
  const size = sizeFor(prefs.shape, d.width, d.height);
  const fps = d.fps ?? 16;
  const defaultSeconds = d.frames ? (d.frames - 1) / fps : 2;
  return (
    <div className="tune-inner">
      <div className="tune-head">
        <h3>{video ? "Video settings" : "Picture settings"}</h3>
        <button className="icon-btn" aria-label="Close" onClick={onClose}>
          <Icon name="close" />
        </button>
      </div>

      <Field label="Shape" hint={`${size.width} × ${size.height} pixels`}>
        <div className="segmented">
          {SHAPES.map((s) => (
            <button
              key={s.label}
              className={prefs.shape === s.shape ? "on" : ""}
              onClick={() => set({ shape: s.shape })}
            >
              {s.shape ? s.label : "Default"}
            </button>
          ))}
        </div>
      </Field>

      {video && (
        <Field
          label="Length"
          hint="Longer videos take much longer and need more memory."
        >
          <div className="slider-row">
            <input
              type="range"
              min={1}
              max={5}
              step={0.5}
              value={prefs.seconds ?? defaultSeconds}
              onChange={(e) => set({ seconds: Number(e.target.value) })}
              aria-label="Length in seconds"
            />
            <output>
              {(prefs.seconds ?? defaultSeconds).toFixed(1)}s
              {prefs.seconds === undefined ? " (default)" : ""}
            </output>
          </div>
        </Field>
      )}

      <Field
        label="Steps"
        hint="More steps add detail but take longer. Turbo models need only a few."
      >
        <div className="slider-row">
          <input
            type="range"
            min={1}
            max={Math.max(50, d.steps * 2)}
            step={1}
            value={prefs.steps ?? d.steps}
            onChange={(e) => set({ steps: Number(e.target.value) })}
            aria-label="Steps"
          />
          <output>{prefs.steps ?? `${d.steps} (default)`}</output>
        </div>
      </Field>

      <Field
        label="Follow the description"
        hint="How closely to stick to the words (guidance). Distilled models like FLUX schnell and Z-Image Turbo want 1."
      >
        <div className="slider-row">
          <input
            type="range"
            min={0}
            max={15}
            step={0.5}
            value={prefs.cfgScale ?? d.cfgScale}
            onChange={(e) => set({ cfgScale: Number(e.target.value) })}
            aria-label="Guidance"
          />
          <output>
            {prefs.cfgScale?.toFixed(1) ?? `${d.cfgScale.toFixed(1)} (default)`}
          </output>
        </div>
      </Field>

      <Field
        label="Starting picture"
        hint="When you attach a picture: how much of it to change."
      >
        <div className="slider-row">
          <input
            type="range"
            min={0.1}
            max={1}
            step={0.05}
            value={prefs.strength ?? 0.75}
            onChange={(e) => set({ strength: Number(e.target.value) })}
            aria-label="How much to change"
            disabled={video}
          />
          <output>
            {video
              ? "Videos keep it"
              : `${Math.round((prefs.strength ?? 0.75) * 100)}%`}
          </output>
        </div>
      </Field>

      <Field
        label="Leave out"
        hint="Things you don't want to see (negative prompt)."
      >
        <textarea
          className="instructions"
          rows={2}
          placeholder={
            d.negativePrompt && /^[\x00-\x7f]*$/.test(d.negativePrompt)
              ? d.negativePrompt
              : "For example: blurry, text, extra fingers"
          }
          value={prefs.negativePrompt ?? ""}
          onChange={(e) => set({ negativePrompt: e.target.value || undefined })}
        />
      </Field>

      <Field
        label="Seed"
        hint="The same seed and words make the same picture. Empty picks a new one each time."
      >
        <div className="num-grid">
          <NumberAuto
            label="Seed"
            value={prefs.seed}
            onChange={(v) =>
              set({ seed: v === undefined ? undefined : Math.round(v) })
            }
            step={1}
            min={0}
            max={2 ** 31}
          />
        </div>
      </Field>

      <div className="tune-foot">
        <button onClick={() => setPrefs({})}>Reset to defaults</button>
        <small>Saved in this browser, for every picture and video model.</small>
      </div>
    </div>
  );
}
