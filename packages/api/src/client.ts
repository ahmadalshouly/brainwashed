import type { ChatRequest, ChatResponse, HostInfo, SkillSummary } from "./types";

export interface ClientOptions {
  /** Base URL of the host gateway, e.g. "http://192.168.1.20:7860". */
  baseUrl: string;
  /** Device token issued at pairing time. */
  token?: string;
  fetch?: typeof fetch;
}

export class HostClient {
  private readonly baseUrl: string;
  private readonly token?: string;
  private readonly fetchImpl: typeof fetch;

  constructor(opts: ClientOptions) {
    this.baseUrl = opts.baseUrl.replace(/\/+$/, "");
    this.token = opts.token;
    this.fetchImpl = opts.fetch ?? globalThis.fetch.bind(globalThis);
  }

  info(): Promise<HostInfo> {
    return this.request<HostInfo>("GET", "/api/info");
  }

  skills(): Promise<SkillSummary[]> {
    return this.request<SkillSummary[]>("GET", "/api/skills");
  }

  chat(req: ChatRequest): Promise<ChatResponse> {
    return this.request<ChatResponse>("POST", "/api/chat", req);
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { Accept: "application/json" };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (this.token) headers.Authorization = `Bearer ${this.token}`;

    const res = await this.fetchImpl(this.baseUrl + path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) {
      throw new HostError(res.status, await res.text());
    }
    return (await res.json()) as T;
  }
}

export class HostError extends Error {
  constructor(
    readonly status: number,
    readonly body: string,
  ) {
    super(`Host request failed with ${status}: ${body}`);
    this.name = "HostError";
  }
}
