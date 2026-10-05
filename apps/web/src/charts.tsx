// Small SVG charts for the admin pages: columns (stacked or single) and a
// line, over time buckets, with a hover readout and keyboard focus.
import { useEffect, useRef, useState, type ReactNode } from "react";

export interface Series<T> {
  label: string;
  /** A CSS color, normally var(--series-N). */
  color: string;
  value: (d: T) => number | null;
}

interface ChartProps<T> {
  data: T[];
  series: Series<T>[];
  /** Label of each point on the x axis and in the readout. */
  label: (d: T, short: boolean) => string;
  format: (n: number) => string;
  height?: number;
  /** Extra readout rows under the series values. */
  extra?: (d: T) => ReactNode;
}

const PAD = { top: 10, right: 8, bottom: 22, left: 44 };

/** Rounds up to 1, 2, 2.5 or 5 times a power of ten. */
export function niceMax(n: number): number {
  if (n <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(n));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= n) return m * p;
  return 10 * p;
}

/** 1,284 / 12.9K / 4.2M */
export function compact(n: number): string {
  if (Math.abs(n) >= 1e6) return `${trim(n / 1e6)}M`;
  if (Math.abs(n) >= 1e4) return `${trim(n / 1e3)}K`;
  // Half of a small axis maximum, like 2.5, keeps its decimal.
  if (Math.abs(n) < 10 && !Number.isInteger(n)) return n.toFixed(1);
  return Math.round(n).toLocaleString();
}

function trim(n: number) {
  return n >= 100 ? Math.round(n).toString() : n.toFixed(1).replace(/\.0$/, "");
}

function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(600);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(240, Math.floor(e.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

function Frame<T>({
  props,
  stacked,
  draw,
}: {
  props: ChartProps<T>;
  stacked: boolean;
  draw: (x: (i: number) => number, y: (v: number) => number, band: number) => ReactNode;
}) {
  const { data, series, label, format, height = 180, extra } = props;
  const [ref, width] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const plotW = width - PAD.left - PAD.right;
  const plotH = height - PAD.top - PAD.bottom;
  const band = data.length ? plotW / data.length : plotW;
  const totals = data.map((d) =>
    stacked
      ? series.reduce((sum, s) => sum + (s.value(d) ?? 0), 0)
      : Math.max(0, ...series.map((s) => s.value(d) ?? 0)),
  );
  const max = niceMax(Math.max(0, ...totals));
  const x = (i: number) => PAD.left + band * i + band / 2;
  const y = (v: number) => PAD.top + plotH - (v / max) * plotH;
  const ticks = [0, max / 2, max];
  // About one label per 70px, always including the last.
  const every = Math.max(1, Math.ceil(data.length / Math.max(1, Math.floor(plotW / 70))));
  const tip = hover !== null ? data[hover] : null;

  return (
    <div className="chart" ref={ref}>
      <svg
        width={width}
        height={height}
        role="img"
        aria-label={series.map((s) => s.label).join(", ")}
        onPointerLeave={() => setHover(null)}
      >
        {ticks.map((t) => (
          <g key={t}>
            <line className="chart-grid" x1={PAD.left} x2={width - PAD.right} y1={y(t)} y2={y(t)} />
            <text className="chart-tick" x={PAD.left - 6} y={y(t)} dy="0.32em" textAnchor="end">
              {format(t)}
            </text>
          </g>
        ))}
        {data.map((d, i) =>
          (data.length - 1 - i) % every === 0 ? (
            <text key={i} className="chart-tick" x={x(i)} y={height - 6} textAnchor="middle">
              {label(d, true)}
            </text>
          ) : null,
        )}
        {hover !== null && (
          <line className="chart-cross" x1={x(hover)} x2={x(hover)} y1={PAD.top} y2={PAD.top + plotH} />
        )}
        {draw(x, y, band)}
        {data.map((d, i) => (
          <rect
            key={i}
            className="chart-hit"
            x={PAD.left + band * i}
            y={PAD.top}
            width={band}
            height={plotH}
            tabIndex={0}
            aria-label={`${label(d, false)}: ${series.map((s) => `${s.label} ${fmt(s.value(d), format)}`).join(", ")}`}
            onPointerMove={() => setHover(i)}
            onFocus={() => setHover(i)}
            onBlur={() => setHover(null)}
          />
        ))}
      </svg>
      {tip && hover !== null && (
        <div
          className="chart-tip"
          style={{
            left: Math.min(Math.max(x(hover), 70), width - 70),
            top: 0,
          }}
        >
          <div className="muted small">{label(tip, false)}</div>
          {series.map((s) => (
            <div key={s.label} className="chart-tip-row">
              <span className="chart-key" style={{ background: s.color }} />
              <strong>{fmt(s.value(tip), format)}</strong>
              <span className="muted">{s.label}</span>
            </div>
          ))}
          {extra?.(tip)}
        </div>
      )}
    </div>
  );
}

function fmt(v: number | null, format: (n: number) => string) {
  return v === null ? "–" : format(v);
}

/** Columns from one baseline; several series stack, with a 2px gap between. */
export function ColumnChart<T>(props: ChartProps<T>) {
  const { data, series } = props;
  return (
    <Frame
      props={props}
      stacked
      draw={(x, y, band) => {
        const w = Math.max(2, Math.min(24, band - 4));
        return data.map((d, i) => {
          let base = 0;
          const top = series.reduce((sum, s) => sum + (s.value(d) ?? 0), 0);
          return series.map((s, k) => {
            const v = s.value(d) ?? 0;
            if (v <= 0) return null;
            const y0 = y(base);
            base += v;
            const y1 = y(base);
            // The gap sits above each segment except the top one.
            const isTop = base === top;
            const h = Math.max(1, y0 - y1 - (isTop ? 0 : 2));
            const r = isTop ? Math.min(4, w / 2, h) : 0;
            const left = x(i) - w / 2;
            const yTop = y1 + (isTop ? 0 : 2);
            return (
              <path
                key={k}
                fill={s.color}
                d={`M${left},${y0} V${yTop + r} Q${left},${yTop} ${left + r},${yTop} H${left + w - r} Q${left + w},${yTop} ${left + w},${yTop + r} V${y0} Z`}
              />
            );
          });
        });
      }}
    />
  );
}

/** A 2px line per series; missing values break the line. */
export function LineChart<T>(props: ChartProps<T>) {
  const { data, series } = props;
  return (
    <Frame
      props={props}
      stacked={false}
      draw={(x, y) =>
        series.map((s) => {
          let d = "";
          let pen = false;
          const dots: ReactNode[] = [];
          data.forEach((p, i) => {
            const v = s.value(p);
            if (v === null) {
              pen = false;
              return;
            }
            d += `${pen ? "L" : "M"}${x(i)},${y(v)} `;
            const prev = i > 0 ? s.value(data[i - 1]) : null;
            const next = i < data.length - 1 ? s.value(data[i + 1]) : null;
            // A point with no neighbours would be invisible as a line.
            if (prev === null && next === null)
              dots.push(<circle key={i} className="chart-dot" cx={x(i)} cy={y(v)} r={4} fill={s.color} />);
            pen = true;
          });
          return (
            <g key={s.label}>
              <path d={d} fill="none" stroke={s.color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
              {dots}
            </g>
          );
        })
      }
    />
  );
}

/** Legend for two or more series: a swatch shaped like the mark. */
export function Legend({ items, line }: { items: { label: string; color: string }[]; line?: boolean }) {
  return (
    <div className="chart-legend">
      {items.map((i) => (
        <span key={i.label}>
          <span className={line ? "chart-key line" : "chart-key"} style={{ background: i.color }} />
          {i.label}
        </span>
      ))}
    </div>
  );
}
