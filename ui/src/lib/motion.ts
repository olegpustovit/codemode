import { flushSync } from "react-dom"

let active: ViewTransition | undefined

export function navigationMotion(type: string) {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ? false
    : { types: [type] }
}

export function move(type: string, update: () => void) {
  if (
    typeof document.startViewTransition !== "function" ||
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  ) {
    update()
    return
  }

  active?.skipTransition()
  const transition = document.startViewTransition({
    types: [type],
    update: () => flushSync(update),
  })
  active = transition
  const clear = () => {
    if (active === transition) active = undefined
  }
  void transition.finished.then(clear, clear)
}
