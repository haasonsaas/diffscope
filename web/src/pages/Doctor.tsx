import { useState } from 'react'
import { Stethoscope, RefreshCw, CheckCircle, XCircle, Cpu, Zap, BrainCircuit } from 'lucide-react'
import { api } from '../api/client'
import type { DoctorResponse } from '../api/types'

export function Doctor() {
  const [result, setResult] = useState<DoctorResponse | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const runDiagnostics = async () => {
    setLoading(true)
    setError(null)
    try {
      const data = await api.getDoctor()
      setResult(data)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Unknown error')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="p-6 max-w-3xl mx-auto">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-lg font-semibold text-text-primary flex items-center gap-2">
          <Stethoscope size={20} className="text-accent" />
          Doctor
        </h2>
        <button
          onClick={runDiagnostics}
          disabled={loading}
          className="flex items-center gap-2 px-3 py-1.5 bg-accent text-white rounded text-[12px] font-medium hover:bg-accent-dim disabled:opacity-50 transition-colors"
        >
          <RefreshCw size={14} className={loading ? 'animate-spin' : ''} />
          Run Diagnostics
        </button>
      </div>

      {error && (
        <div className="bg-sev-error/5 border border-sev-error/20 rounded-lg p-3 mb-4 text-[12px] text-sev-error">
          {error}
        </div>
      )}

      {!result && !loading && (
        <div className="bg-surface-1 border border-border rounded-lg p-12 text-center text-text-muted text-sm">
          Click "Run Diagnostics" to check your setup
        </div>
      )}

      {result && (
        <div className="space-y-3">
          {/* Config */}
          <div className="bg-surface-1 border border-border rounded-lg p-4">
            <h3 className="text-[11px] font-semibold text-text-muted uppercase tracking-wider mb-3">Configuration</h3>
            <div className="grid grid-cols-2 gap-3 text-[12px]">
              {[
                ['Model', result.config.model],
                ['Adapter', result.config.adapter || 'auto'],
                ['Base URL', result.config.base_url],
                ['API Key', result.config.api_key_set ? '***' : 'not set'],
              ].map(([label, value]) => (
                <div key={label} className="flex justify-between">
                  <span className="text-text-muted">{label}</span>
                  <span className="font-code text-text-primary truncate ml-4">{value}</span>
                </div>
              ))}
            </div>
          </div>

          {/* Endpoint */}
          <div className="bg-surface-1 border border-border rounded-lg p-4">
            <h3 className="text-[11px] font-semibold text-text-muted uppercase tracking-wider mb-3">Endpoint</h3>
            <div className="flex items-center gap-2">
              {result.endpoint_reachable ? (
                <>
                  <CheckCircle className="text-sev-suggestion" size={16} />
                  <span className="text-sev-suggestion text-[13px]">
                    Reachable ({result.endpoint_type})
                  </span>
                </>
              ) : (
                <>
                  <XCircle className="text-sev-error" size={16} />
                  <span className="text-sev-error text-[13px]">Unreachable</span>
                </>
              )}
            </div>
          </div>

          {result.learning && (
            <div className="bg-surface-1 border border-border rounded-lg p-4">
              <h3 className="text-[11px] font-semibold text-text-muted uppercase tracking-wider mb-3 flex items-center gap-2">
                <BrainCircuit size={13} />
                Learning & Evidence
              </h3>
              <div className="grid grid-cols-2 gap-3 text-[12px]">
                {[
                  ['Aggregate feedback', result.learning.enhanced_feedback ? 'on' : 'off'],
                  ['Semantic feedback', result.learning.semantic_feedback ? 'on' : 'off'],
                  ['Semantic retrieval', result.learning.semantic_rag ? 'on' : 'off'],
                  ['Feedback store', result.learning.feedback_store_exists ? 'present' : 'empty'],
                  ['Semantic store', result.learning.semantic_feedback_store_exists ? 'present' : 'empty'],
                  ['Min observations', String(result.learning.min_feedback_observations)],
                  ['Min semantic examples', String(result.learning.semantic_feedback_min_examples)],
                  ['Semantic similarity', String(result.learning.semantic_feedback_similarity)],
                ].map(([label, value]) => (
                  <div key={label} className="flex justify-between gap-3">
                    <span className="text-text-muted">{label}</span>
                    <span className="font-code text-text-primary truncate">{value}</span>
                  </div>
                ))}
              </div>
              <div className="mt-3 space-y-1 text-[11px] font-code text-text-muted">
                <div className="truncate">{result.learning.feedback_path}</div>
                <div className="truncate">{result.learning.semantic_feedback_path}</div>
              </div>
            </div>
          )}

          {/* Models */}
          {result.models.length > 0 && (
            <div className="bg-surface-1 border border-border rounded-lg p-4">
              <h3 className="text-[11px] font-semibold text-text-muted uppercase tracking-wider mb-3">
                Available Models ({result.models.length})
              </h3>
              <div className="space-y-1.5">
                {result.models.map((m) => {
                  const isRecommended = m.name === result.recommended_model
                  return (
                    <div
                      key={m.name}
                      className={`flex items-center gap-3 px-3 py-2 rounded text-[12px] ${
                        isRecommended ? 'bg-accent/5 border border-accent/20' : 'bg-surface-2'
                      }`}
                    >
                      <Cpu size={13} className="text-text-muted shrink-0" />
                      <span className="font-code text-text-primary flex-1">{m.name}</span>
                      {m.size_mb > 0 && (
                        <span className="text-[11px] text-text-muted">{(m.size_mb / 1024).toFixed(1)} GB</span>
                      )}
                      {m.quantization && (
                        <span className="text-[10px] bg-surface-3 text-text-muted px-1.5 py-0.5 rounded font-code">
                          {m.quantization}
                        </span>
                      )}
                      {isRecommended && (
                        <span className="flex items-center gap-1 text-[10px] bg-accent/10 text-accent px-2 py-0.5 rounded font-medium">
                          <Zap size={10} />
                          Recommended
                        </span>
                      )}
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
