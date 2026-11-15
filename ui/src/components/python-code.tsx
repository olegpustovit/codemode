const tokens =
  /("""[\s\S]*?"""|'''[\s\S]*?'''|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|#[^\n]*|\b(?:await|return|for|in|async|def)\b)/g
const keyword = /^(await|return|for|in|async|def)$/
export function PythonCode({ code }: { code: string }) {
  return (
    <code>
      {code.split(tokens).map((part, index) =>
        part.startsWith("#") ? (
          <span className="code-comment" key={index}>
            {part}
          </span>
        ) : keyword.test(part) ? (
          <span className="code-keyword" key={index}>
            {part}
          </span>
        ) : (
          part
        )
      )}
    </code>
  )
}
