import { useState, useCallback } from 'react'

// ======================================================================
// Types
// ======================================================================

interface BenchRow {
  name: string
  group: string
  staticNs: number
  dynamicNs: number
}

// ======================================================================
// Benchmark runner
// ======================================================================

async function measure(channel: string, iterations: number): Promise<number> {
  // Warmup
  const warmup = Math.min(iterations, 50)
  for (let i = 0; i < warmup; i++) await window.api.ipc(channel)

  const start = performance.now()
  for (let i = 0; i < iterations; i++) await window.api.ipc(channel)
  const totalMs = performance.now() - start
  return (totalMs * 1_000_000) / iterations // ns per call
}

interface BenchDef {
  name: string
  group: string
  staticChannel: string
  dynamicChannel: string
}

const BENCHMARKS: BenchDef[] = [
  // Getter (0 in → 1 out)
  { name: 'get_Host → hstring', group: 'Getter (0 in → 1 out)', staticChannel: 'static-get-host', dynamicChannel: 'dynamic-get-host' },
  { name: 'get_Port → i32', group: 'Getter (0 in → 1 out)', staticChannel: 'static-get-port', dynamicChannel: 'dynamic-get-port' },
  { name: 'get_Suspicious → bool', group: 'Getter (0 in → 1 out)', staticChannel: 'static-get-suspicious', dynamicChannel: 'dynamic-get-suspicious' },
  // NOTE: get_QueryParsed disabled — WwwFormUrlDecoder COM Release in napi External
  // finalizer segfaults during V8 GC in Electron's async IPC loop. Works fine in non-IPC bench.
  // { name: 'get_QueryParsed → object', group: 'Getter (0 in → 1 out)', staticChannel: 'static-get-query-parsed', dynamicChannel: 'dynamic-get-query-parsed' },
  // Factory (1 in → 1 out)
  { name: 'CreateUri (hstring)', group: 'Factory (1 in → 1 out)', staticChannel: 'static-create-uri', dynamicChannel: 'dynamic-create-uri' },
  { name: 'PV.CreateInt32 (i32)', group: 'Factory (1 in → 1 out)', staticChannel: 'static-pv-i32', dynamicChannel: 'dynamic-pv-i32' },
  { name: 'PV.CreateDouble (f64)', group: 'Factory (1 in → 1 out)', staticChannel: 'static-pv-f64', dynamicChannel: 'dynamic-pv-f64' },
  { name: 'PV.CreateBoolean (bool)', group: 'Factory (1 in → 1 out)', staticChannel: 'static-pv-bool', dynamicChannel: 'dynamic-pv-bool' },
  { name: 'PV.CreateString (hstring)', group: 'Factory (1 in → 1 out)', staticChannel: 'static-pv-string', dynamicChannel: 'dynamic-pv-string' },
  // Struct (struct → 1 out)
  { name: 'Geopoint.Create (3×f64 struct)', group: 'Factory (1 in → 1 out)', staticChannel: 'static-geopoint', dynamicChannel: 'dynamic-geopoint' },
  // Method (1 in → 1 out)
  { name: 'CombineUri (hstring)', group: 'Method (1 in → 1 out)', staticChannel: 'static-combine-uri', dynamicChannel: 'dynamic-combine-uri' },
]

// ======================================================================
// Formatting
// ======================================================================

function fmt(ns: number): string {
  if (ns >= 1_000_000) return `${(ns / 1_000_000).toFixed(2)} ms`
  if (ns >= 1000) return `${(ns / 1000).toFixed(2)} µs`
  return `${ns.toFixed(0)} ns`
}

function fmtRatio(staticNs: number, dynamicNs: number): string {
  if (staticNs <= 0) return '-'
  return `${(dynamicNs / staticNs).toFixed(1)}x`
}

function fmtOverhead(staticNs: number, dynamicNs: number): string {
  const diff = dynamicNs - staticNs
  return `+${fmt(diff)}`
}

// ======================================================================
// Component
// ======================================================================

function App(): JSX.Element {
  const [rows, setRows] = useState<BenchRow[]>([])
  const [noopNs, setNoopNs] = useState<number | null>(null)
  const [status, setStatus] = useState<'idle' | 'running' | 'done'>('idle')
  const [progress, setProgress] = useState('')
  const [iterations, setIterations] = useState(1000)

  const run = useCallback(async () => {
    setStatus('running')
    setRows([])
    setNoopNs(null)

    try {
      // IPC baseline
      setProgress('Measuring IPC baseline...')
      const noop = await measure('ipc-noop', iterations)
      setNoopNs(noop)

      const results: BenchRow[] = []

      for (const b of BENCHMARKS) {
        setProgress(`${b.name} (static)...`)
        const s = await measure(b.staticChannel, iterations)
        setProgress(`${b.name} (dynamic)...`)
        const d = await measure(b.dynamicChannel, iterations)
        results.push({ name: b.name, group: b.group, staticNs: s, dynamicNs: d })
        setRows([...results])
      }

      // Log to main console
      const lines = [
        `[bench] IPC noop: ${fmt(noop)}`,
        ...results.map(
          (r) =>
            `[bench] ${r.name}: static=${fmt(r.staticNs)}  dynamic=${fmt(r.dynamicNs)}  ratio=${fmtRatio(r.staticNs, r.dynamicNs)}  overhead=${fmtOverhead(r.staticNs, r.dynamicNs)}`
        )
      ]
      await window.api.logResults(lines)

      setStatus('done')
      setProgress('')
    } catch (err) {
      setStatus('idle')
      setProgress(`Error: ${err}`)
    }
  }, [iterations])

  // Group rows
  const groups: [string, BenchRow[]][] = []
  const seen = new Set<string>()
  for (const r of rows) {
    if (!seen.has(r.group)) {
      seen.add(r.group)
      groups.push([r.group, []])
    }
    groups.find(([g]) => g === r.group)![1].push(r)
  }

  return (
    <div style={styles.container}>
      <h1 style={styles.title}>dynwinrt Electron Benchmark</h1>
      <p style={styles.subtitle}>
        Every call: Renderer → IPC → Main Process → native addon → WinRT → IPC → Renderer
      </p>

      <div style={styles.controls}>
        <label style={styles.label}>
          Iterations:
          <input
            type="number"
            value={iterations}
            onChange={(e) => setIterations(Math.max(10, parseInt(e.target.value) || 100))}
            style={styles.input}
            disabled={status === 'running'}
            min={10}
            step={100}
          />
        </label>
        <button
          onClick={run}
          disabled={status === 'running'}
          style={{ ...styles.button, opacity: status === 'running' ? 0.6 : 1 }}
        >
          {status === 'running' ? 'Running...' : 'Run Benchmark'}
        </button>
        <span style={styles.status}>
          {status === 'running' && progress}
          {status === 'done' && 'Complete'}
        </span>
      </div>

      {noopNs !== null && (
        <div style={styles.baseline}>IPC baseline (noop round-trip): <strong>{fmt(noopNs)}</strong></div>
      )}

      {groups.length > 0 && (
        <div style={styles.resultsContainer}>
          {groups.map(([groupName, items]) => (
            <div key={groupName} style={styles.groupSection}>
              <h3 style={styles.groupTitle}>{groupName}</h3>
              <table style={styles.table}>
                <thead>
                  <tr>
                    <th style={styles.th}>Operation</th>
                    <th style={styles.thRight}>Static (C++/WinRT)</th>
                    <th style={styles.thRight}>Dynamic (dynwinrt)</th>
                    <th style={styles.thRight}>Ratio</th>
                    <th style={styles.thRight}>Overhead</th>
                  </tr>
                </thead>
                <tbody>
                  {items.map((r, i) => (
                    <tr key={i} style={i % 2 === 0 ? styles.rowEven : styles.rowOdd}>
                      <td style={styles.td}>{r.name}</td>
                      <td style={styles.tdNum}>{fmt(r.staticNs)}</td>
                      <td style={styles.tdNum}>{fmt(r.dynamicNs)}</td>
                      <td style={styles.tdNum}>{fmtRatio(r.staticNs, r.dynamicNs)}</td>
                      <td style={styles.tdNum}>{fmtOverhead(r.staticNs, r.dynamicNs)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ))}
        </div>
      )}

      {status === 'idle' && rows.length === 0 && (
        <p style={styles.hint}>Click "Run Benchmark" to measure full Electron IPC + WinRT round-trip performance.</p>
      )}
    </div>
  )
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif',
    maxWidth: 920,
    margin: '0 auto',
    padding: '24px 20px',
    color: '#e0e0e0',
    backgroundColor: '#1e1e1e',
    minHeight: '100vh',
    boxSizing: 'border-box'
  },
  title: { fontSize: 22, fontWeight: 700, margin: '0 0 4px 0', color: '#ffffff' },
  subtitle: { fontSize: 13, color: '#999', margin: '0 0 20px 0' },
  controls: { display: 'flex', alignItems: 'center', gap: 16, marginBottom: 16, flexWrap: 'wrap' as const },
  label: { fontSize: 14, display: 'flex', alignItems: 'center', gap: 8 },
  input: { width: 80, padding: '6px 8px', fontSize: 14, border: '1px solid #555', borderRadius: 4, backgroundColor: '#2d2d2d', color: '#e0e0e0' },
  button: { padding: '8px 20px', fontSize: 14, fontWeight: 600, border: 'none', borderRadius: 4, backgroundColor: '#0078d4', color: '#fff', cursor: 'pointer' },
  status: { fontSize: 13, color: '#999' },
  baseline: { fontSize: 13, color: '#aaa', marginBottom: 16, padding: '8px 12px', backgroundColor: '#252525', borderRadius: 4, borderLeft: '3px solid #0078d4' },
  resultsContainer: { marginTop: 4 },
  groupSection: { marginBottom: 20 },
  groupTitle: { fontSize: 15, fontWeight: 600, margin: '0 0 8px 0', color: '#ccc', borderBottom: '1px solid #333', paddingBottom: 4 },
  table: { width: '100%', borderCollapse: 'collapse' as const, fontSize: 13 },
  th: { textAlign: 'left' as const, padding: '6px 10px', borderBottom: '2px solid #444', color: '#aaa', fontWeight: 600, fontSize: 12, textTransform: 'uppercase' as const, letterSpacing: '0.5px' },
  thRight: { textAlign: 'right' as const, padding: '6px 10px', borderBottom: '2px solid #444', color: '#aaa', fontWeight: 600, fontSize: 12, textTransform: 'uppercase' as const, letterSpacing: '0.5px' },
  td: { padding: '5px 10px', borderBottom: '1px solid #333' },
  tdNum: { padding: '5px 10px', borderBottom: '1px solid #333', textAlign: 'right' as const, fontWeight: 600, fontVariantNumeric: 'tabular-nums' },
  rowEven: { backgroundColor: '#252525' },
  rowOdd: { backgroundColor: '#1e1e1e' },
  hint: { fontSize: 14, color: '#777', marginTop: 40, textAlign: 'center' as const }
}

export default App
