import { sourceIconKeys, VendorGlyph } from "./vendor-glyph"

export function IconPicker({
  selected,
  label,
}: {
  selected: string
  label: string
}) {
  return (
    <fieldset className="icon-picker">
      <legend>Icon</legend>
      <div>
        {sourceIconKeys.map((key) => (
          <label key={key} title={key === "mono" ? "Monogram" : key}>
            <input
              type="radio"
              name="icon"
              value={key}
              defaultChecked={key === selected}
              aria-label={key === "mono" ? "Monogram" : key}
            />
            <span>
              <VendorGlyph name={key} label={label} />
            </span>
          </label>
        ))}
      </div>
    </fieldset>
  )
}
