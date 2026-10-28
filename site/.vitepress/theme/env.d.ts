declare const __OT_VERSION__: string
declare const __OT_REPO__: string

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent
  export default component
}
