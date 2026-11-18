import { flushSync } from "react-dom"

let active: ViewTransition | undefined

const reducedMotion = () =>
  window.matchMedia("(prefers-reduced-motion: reduce)").matches

export function navigationMotion(type: string) {
  return reducedMotion()
    ? false
    : { types: (): string[] | false => (reducedMotion() ? false : [type]) }
}

export function move(type: string, update: () => void) {
  if (typeof document.startViewTransition !== "function" || reducedMotion()) {
    update()
    return
  }

  active?.skipTransition()
  type TransitionScope = Element & {
    startViewTransition?: Document["startViewTransition"]
  }
  const scope = document.querySelector?.<TransitionScope>(".pane-stack")
  const options = { types: [type], update: () => flushSync(update) }
  const transition =
    scope?.startViewTransition?.(options) ??
    document.startViewTransition(options)
  void transition.ready?.catch(() => {})
  active = transition
  const clear = () => {
    if (active === transition) active = undefined
  }
  void transition.finished.then(clear, clear)
}
