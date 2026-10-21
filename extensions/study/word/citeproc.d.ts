declare module 'citeproc' {
  export class Engine {
    constructor(sys: { retrieveLocale: (lang: string) => string; retrieveItem: (id: string) => unknown }, style: string, lang?: string)
    setOutputFormat(format: string): void
    rebuildProcessorState(citations: unknown[], mode?: string): [string, number, string][]
    makeBibliography(): [Record<string, unknown>, string[]] | false
  }
}
