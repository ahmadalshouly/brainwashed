import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { fetch as expoFetch } from "expo/fetch";
import { RemoteHost, type PairedHost } from "@brainwashed/api";
import { deleteItem, getItem, setItem } from "./storage";

// Each host is stored under its own key; an index lists the ids. Keychain
// entries are kept small this way.
const INDEX_KEY = "brainwashed.hosts";
const hostKey = (id: string) => `brainwashed.host.${id}`;

export const remoteFetch = expoFetch as unknown as ConstructorParameters<typeof RemoteHost>[1];

interface HostsContext {
  hosts: PairedHost[] | null;
  addHost(host: PairedHost): Promise<void>;
  removeHost(hostId: string): Promise<void>;
  remote(hostId: string): RemoteHost | null;
}

const Ctx = createContext<HostsContext | null>(null);

export function HostsProvider({ children }: { children: ReactNode }) {
  const [hosts, setHosts] = useState<PairedHost[] | null>(null);

  useEffect(() => {
    (async () => {
      const ids: string[] = JSON.parse((await getItem(INDEX_KEY)) ?? "[]");
      const loaded = await Promise.all(ids.map(async (id) => JSON.parse((await getItem(hostKey(id))) ?? "null")));
      setHosts(loaded.filter(Boolean));
    })().catch(() => setHosts([]));
  }, []);

  const save = useCallback(async (next: PairedHost[]) => {
    setHosts(next);
    await Promise.all(next.map((h) => setItem(hostKey(h.hostId), JSON.stringify(h))));
    await setItem(INDEX_KEY, JSON.stringify(next.map((h) => h.hostId)));
  }, []);

  const value = useMemo<HostsContext>(
    () => ({
      hosts,
      async addHost(host) {
        // Re-pairing the same computer replaces the old record.
        await save([...(hosts ?? []).filter((h) => h.hostId !== host.hostId), host]);
      },
      async removeHost(hostId) {
        await save((hosts ?? []).filter((h) => h.hostId !== hostId));
        await deleteItem(hostKey(hostId));
      },
      remote(hostId) {
        const host = hosts?.find((h) => h.hostId === hostId);
        if (!host) return null;
        return new RemoteHost({ ...host }, remoteFetch, (address) => {
          // Remember the address that worked so the next call tries it first.
          setItem(hostKey(hostId), JSON.stringify({ ...host, lastAddress: address }));
        });
      },
    }),
    [hosts, save],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useHosts(): HostsContext {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useHosts outside HostsProvider");
  return ctx;
}
