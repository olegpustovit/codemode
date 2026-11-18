import type { ReactNode } from "react"

const tokens =
  /(?<comment>#[^\n]*)|(?<string>\b[rbfuRBFU]{1,2}(?=["'])|)(?:"""(?:\\[\s\S]|(?!""")[\s\S])*"""|'''(?:\\[\s\S]|(?!''')[\s\S])*'''|"(?:\\[^\n]|[^"\\\n])*"|'(?:\\[^\n]|[^'\\\n])*')|(?<tool>\btools\.[A-Za-z_]\w*\.[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)|(?<number>\b\d[\d_]*(?:\.\d[\d_]*)?)|(?<identifier>\b[A-Za-z_]\w*)/g
const keywords = new Set(
  "False None True and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield".split(
    " "
  )
)
const builtins = new Set(
  "print len set dict list int str sorted sum max min any all isinstance range enumerate zip json Decimal ToolError abs round".split(
    " "
  )
)

export function PythonCode({
  code,
  onTool,
}: {
  code: string
  onTool?: (path: string) => void
}) {
  const lines: ReactNode[][] = [[]]
  function append(value: string, kind?: string, tool?: string) {
    value.split("\n").forEach((part, index) => {
      if (index > 0) lines.push([])
      if (!part) return
      const line = lines[lines.length - 1]
      if (tool && onTool) {
        const path = tool.slice("tools.".length).replace(/\.all$/, "")
        line.push(
          <button
            type="button"
            key={line.length}
            className="code-tool"
            title={`Jump to calls of ${path}`}
            onClick={() => onTool(path)}
          >
            {part}
          </button>
        )
      } else {
        line.push(
          kind ? (
            <span key={line.length} className={`code-${kind}`}>
              {part}
            </span>
          ) : (
            part
          )
        )
      }
    })
  }

  let end = 0
  for (const match of code.matchAll(tokens)) {
    append(code.slice(end, match.index))
    const { comment, string, tool, number, identifier } = match.groups!
    const kind = comment
      ? "comment"
      : string !== undefined
        ? "string"
        : number
          ? "number"
          : keywords.has(identifier)
            ? "keyword"
            : builtins.has(identifier)
              ? "builtin"
              : undefined
    append(match[0], kind, tool)
    end = match.index + match[0].length
  }
  append(code.slice(end))

  return (
    <code className="python-code">
      {lines.map((line, index) => (
        <span className="ln" key={index}>
          {line}
          {index + 1 < lines.length ? "\n" : null}
        </span>
      ))}
    </code>
  )
}
