import { ref } from 'vue'

export type LogKind = 'buffer' | 'hit' | 'consume' | 'replay' | 'pass' | 'cancel'

const labels: Record<LogKind, string> = {
  buffer: '暂存',
  hit: '命中',
  consume: '消费',
  replay: '回放',
  pass: '透传',
  cancel: '取消',
}

export interface LogEntry {
  id: number
  kind: LogKind
  label: string
  text: string
}

/** 演示用的引擎决策记录，只保留最近几条。 */
export function useEventLog(limit = 7) {
  const entries = ref<LogEntry[]>([])
  let seq = 0
  function push(kind: LogKind, text: string) {
    entries.value = [{ id: ++seq, kind, label: labels[kind], text }, ...entries.value].slice(0, limit)
  }
  function clear() {
    entries.value = []
  }
  return { entries, push, clear }
}
