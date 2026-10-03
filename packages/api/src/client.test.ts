import { describe, expect, it, vi } from "vitest";
import { HostClient, HostError } from "./client";

function mockFetch(status: number, body: unknown) {
  return vi.fn(async () => new Response(JSON.stringify(body), { status }));
}

describe("HostClient", () => {
  it("sends the device token and parses JSON", async () => {
    const fetch = mockFetch(200, { name: "laptop", version: "0.0.0", model: null });
    const client = new HostClient({ baseUrl: "http://host:7860/", token: "abc", fetch });

    const info = await client.info();

    expect(info.name).toBe("laptop");
    const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("http://host:7860/api/info");
    expect((init.headers as Record<string, string>).Authorization).toBe("Bearer abc");
  });

  it("throws HostError on non-2xx", async () => {
    const client = new HostClient({ baseUrl: "http://host", fetch: mockFetch(401, "nope") });
    await expect(client.skills()).rejects.toBeInstanceOf(HostError);
  });
});
