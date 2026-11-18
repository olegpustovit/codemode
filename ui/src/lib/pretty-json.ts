export function prettyJson(text: string) {
  let output = ""
  let depth = 0
  let quoted = false
  let escaped = false
  const indent = () => "\n" + "  ".repeat(Math.min(depth, 32))

  for (const char of text) {
    if (quoted) {
      output += char
      if (escaped) escaped = false
      else if (char === "\\") escaped = true
      else if (char === '"') quoted = false
    } else if (char === '"') {
      quoted = true
      output += char
    } else if (char === "{" || char === "[") {
      depth++
      output += char + indent()
    } else if (char === "}" || char === "]") {
      depth = Math.max(0, depth - 1)
      output += indent() + char
    } else if (char === ",") {
      output += char + indent()
    } else if (char === ":") {
      output += ": "
    } else if (!/\s/.test(char)) {
      output += char
    }
  }
  return output
}
