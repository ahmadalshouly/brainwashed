import { useState } from "react";
import type { ApiKey, ApiUsage, UsageBucket, UsageGroup } from "@brainwashed/api";
import { ColumnChart, compact, Legend, LineChart } from "../charts";
import { Card, useHost, useLoad } from "../ui";

const RANGES = [
  { days: 1, label: "24 hours" },
  { days: 7, label: "7 days" },
  { days: 30, label: "30 days" },
  { days: 90, label: "90 days" },
];

const IN = "var(--series-1)";
const OUT = "var(--series-2)";

function seconds(ms: number | null) {
  if (ms === null) return "–";
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

function speed(tps: number) {
  return tps > 0 ? `${tps.toFixed(tps < 10 ? 1 : 0)} tok/s` : "–";
}

function Tile({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="tile">
      <div className="muted small">{label}</div>
      <div className="tile-value">{value}</div>
      {note && <div className="muted small">{note}</div>}
    </div>
  );
}

function Breakdown({ title, groups, name }: { title: string; groups: UsageGroup[]; name: (id: string) => string }) {
  const total = groups.reduce((s, g) => s + g.promptTokens + g.completionTokens, 0) || 1;
  return (
    <Card title={title}>
      {groups.length === 0 ? (
        <p className="muted">Nothing yet in this period.</p>
      ) : (
        <table className="table usage-table">
          <thead>
            <tr>
              <th>Name</th>
              <th className="right">Requests</th>
              <th className="right">Tokens in</th>
              <th className="right">Tokens out</th>
              <th className="right">Speed</th>
              <th>Share of tokens</th>
            </tr>
          </thead>
          <tbody>
            {groups.map((g) => {
              const share = (g.promptTokens + g.completionTokens) / total;
              return (
                <tr key={g.id}>
                  <td>{name(g.id)}</td>
                  <td className="right num">
                    {compact(g.requests)}
                    {g.errors > 0 && <span className="muted small"> ({g.errors} failed)</span>}
                  </td>
                  <td className="right num">{compact(g.promptTokens)}</td>
                  <td className="right num">{compact(g.completionTokens)}</td>
                  <td className="right num">{speed(g.tokensPerSecond)}</td>
                  <td>
                    <div className="share" title={`${Math.round(share * 100)}%`}>
                      <div style={{ width: `${Math.max(share * 100, 1)}%` }} />
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </Card>
  );
}

export function ApiUsageView({ keys }: { keys: ApiKey[] | null | undefined }) {
  const { remote } = useHost();
  const [days, setDays] = useState(7);
  const [asTable, setAsTable] = useState(false);
  const usage = useLoad<ApiUsage>(() => remote.apiUsage(days), [remote, days], 30000);
  const u = usage.value;

  const hourly = (u?.bucket ?? 86400) < 86400;
  const when = (b: UsageBucket, short: boolean) => {
    const d = new Date(b.start * 1000);
    if (hourly)
      return short
        ? d.toLocaleTimeString([], { hour: "numeric" })
        : d.toLocaleString([], {
            weekday: "short",
            hour: "numeric",
            minute: "2-digit",
          });
    return d.toLocaleDateString(
      [],
      short ? { month: "short", day: "numeric" } : { weekday: "short", month: "short", day: "numeric" },
    );
  };
  const keyName = (id: string) => keys?.find((k) => k.id === id)?.name ?? "Revoked key";
  const modelName = (id: string) => (id === "local" ? "Local model" : id);
  const t = u?.totals;
  const series = u?.series ?? [];

  return (
    <div className="usage">
      <div className="row wrap usage-filters" role="group" aria-label="Period">
        {RANGES.map((r) => (
          <button
            key={r.days}
            className={r.days === days ? "on" : ""}
            aria-pressed={r.days === days}
            onClick={() => setDays(r.days)}
          >
            {r.label}
          </button>
        ))}
      </div>

      <div className="tiles">
        <Tile
          label="Requests"
          value={t ? compact(t.requests) : "–"}
          note={t?.errors ? `${t.errors} failed` : undefined}
        />
        <Tile label="Tokens in" value={t ? compact(t.promptTokens) : "–"} />
        <Tile label="Tokens out" value={t ? compact(t.completionTokens) : "–"} />
        <Tile label="Average speed" value={t ? speed(t.tokensPerSecond) : "–"} />
        <Tile label="Time to first word" value={t ? seconds(t.firstTokenMs) : "–"} />
      </div>

      <Card
        title="Tokens"
        actions={
          <button className="ghost small" onClick={() => setAsTable(!asTable)}>
            {asTable ? "Show chart" : "Show table"}
          </button>
        }
      >
        {asTable ? (
          <table className="table usage-table">
            <thead>
              <tr>
                <th>{hourly ? "Hour" : "Day"}</th>
                <th className="right">Requests</th>
                <th className="right">Tokens in</th>
                <th className="right">Tokens out</th>
                <th className="right">Speed</th>
                <th className="right">First word</th>
              </tr>
            </thead>
            <tbody>
              {series
                .filter((b) => b.requests > 0)
                .reverse()
                .map((b) => (
                  <tr key={b.start}>
                    <td>{when(b, false)}</td>
                    <td className="right num">{b.requests.toLocaleString()}</td>
                    <td className="right num">{b.promptTokens.toLocaleString()}</td>
                    <td className="right num">{b.completionTokens.toLocaleString()}</td>
                    <td className="right num">{speed(b.tokensPerSecond)}</td>
                    <td className="right num">{seconds(b.firstTokenMs)}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        ) : (
          <>
            <Legend
              items={[
                { label: "Tokens in", color: IN },
                { label: "Tokens out", color: OUT },
              ]}
            />
            <ColumnChart
              data={series}
              label={when}
              format={compact}
              series={[
                { label: "Tokens in", color: IN, value: (b) => b.promptTokens },
                {
                  label: "Tokens out",
                  color: OUT,
                  value: (b) => b.completionTokens,
                },
              ]}
              extra={(b) => <div className="muted small">{b.requests.toLocaleString()} requests</div>}
            />
          </>
        )}
      </Card>

      <div className="usage-grid">
        <Card title="Requests">
          <ColumnChart
            data={series}
            label={when}
            format={compact}
            height={150}
            series={[{ label: "Requests", color: IN, value: (b) => b.requests }]}
            extra={(b) => (b.errors ? <div className="muted small">{b.errors} failed</div> : null)}
          />
        </Card>
        <Card title="Speed (tokens per second)">
          <LineChart
            data={series}
            label={when}
            format={(n) => compact(n)}
            height={150}
            series={[
              {
                label: "tok/s",
                color: IN,
                value: (b) => (b.tokensPerSecond > 0 ? Math.round(b.tokensPerSecond * 10) / 10 : null),
              },
            ]}
            extra={(b) => <div className="muted small">First word after {seconds(b.firstTokenMs)}</div>}
          />
        </Card>
      </div>

      <Breakdown title="By model" groups={u?.byModel ?? []} name={modelName} />
      <Breakdown title="By key" groups={u?.byKey ?? []} name={keyName} />
      <p className="muted small">
        Counts replies to API keys only, not chats in the app. Kept on this computer for 90 days; message text is never
        stored.
      </p>
    </div>
  );
}
