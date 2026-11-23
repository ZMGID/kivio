/** Native CLI routing aliases can be stored in older conversations and catalogs. */
export function concreteCliModelId(value?: string | null): string | null {
  const id = value?.trim()
  return id && !/(?:^|\/)(?:auto|default)(?:\[[^\]]*\])?$/i.test(id) ? id : null
}

export function isConcreteCliModel(model: { id: string; label?: string }): boolean {
  const label = model.label?.replace(/\s*(?:\([^()]*\)|（[^（）]*）)$/, '').trim()
  return Boolean(concreteCliModelId(model.id) && concreteCliModelId(label || model.id))
}
