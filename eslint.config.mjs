import { plugin as shadcn } from '@shadcn/lint'
import tsParser from '@typescript-eslint/parser'
import tsPlugin from '@typescript-eslint/eslint-plugin'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import globals from 'globals'

export default [
  {
    // Preserve the legacy config's ignores, including ESLint 8's dotfile default.
    ignores: ['**/dist/**', '**/output/**', '**/test-ZMGID/**', '**/.*'],
  },
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      parser: tsParser,
      ecmaVersion: 2020,
      sourceType: 'module',
      parserOptions: { ecmaFeatures: { jsx: true } },
      globals: { ...globals.browser, ...globals.es2020 },
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
      shadcn,
    },
    settings: {
      shadcn: {
        componentImports: [
          '(?:^|/)components/Button$',
          '(?:^|/)settings/(?:public/controls|components)$',
        ],
        note: 'See docs/engineering-standards.md for shared UI and theme rules.',
      },
    },
    rules: {
      // Keep the ESLint 8 / typescript-eslint 7 policy during the required upgrade.
      // Explicit rules avoid adopting new recommended rules from newer versions.
      'no-array-constructor': 'off',
      'no-loss-of-precision': 'off',
      'no-unused-vars': 'off',
      'constructor-super': 'off',
      'getter-return': 'off',
      'no-const-assign': 'off',
      'no-dupe-args': 'off',
      'no-dupe-class-members': 'off',
      'no-dupe-keys': 'off',
      'no-func-assign': 'off',
      'no-import-assign': 'off',
      'no-new-symbol': 'off',
      'no-new-native-nonconstructor': 'off',
      'no-obj-calls': 'off',
      'no-redeclare': 'off',
      'no-setter-return': 'off',
      'no-this-before-super': 'off',
      'no-undef': 'off',
      'no-unreachable': 'off',
      'no-unsafe-negation': 'off',
      'no-var': 'error',
      'prefer-const': 'error',
      'prefer-rest-params': 'error',
      'prefer-spread': 'error',
      'for-direction': 'error',
      'no-async-promise-executor': 'error',
      'no-case-declarations': 'error',
      'no-class-assign': 'error',
      'no-compare-neg-zero': 'error',
      'no-cond-assign': 'error',
      'no-constant-condition': ['error', { checkLoops: 'all' }],
      'no-control-regex': 'error',
      'no-debugger': 'error',
      'no-delete-var': 'error',
      'no-dupe-else-if': 'error',
      'no-duplicate-case': 'error',
      'no-empty': 'error',
      'no-empty-character-class': 'error',
      'no-empty-pattern': 'error',
      'no-ex-assign': 'error',
      'no-extra-boolean-cast': 'error',
      'no-extra-semi': 'error',
      'no-fallthrough': 'error',
      'no-global-assign': 'error',
      'no-inner-declarations': ['error', 'functions', { blockScopedFunctions: 'disallow' }],
      'no-invalid-regexp': 'error',
      'no-irregular-whitespace': 'error',
      'no-misleading-character-class': 'error',
      'no-mixed-spaces-and-tabs': 'error',
      'no-nonoctal-decimal-escape': 'error',
      'no-octal': 'error',
      'no-prototype-builtins': 'error',
      'no-regex-spaces': 'error',
      'no-self-assign': 'error',
      'no-shadow-restricted-names': 'error',
      'no-sparse-arrays': 'error',
      'no-unexpected-multiline': 'error',
      'no-unsafe-finally': 'error',
      'no-unsafe-optional-chaining': 'error',
      'no-unused-labels': 'error',
      'no-useless-backreference': 'error',
      'no-useless-catch': 'error',
      'no-useless-escape': 'error',
      'no-with': 'error',
      'require-yield': 'error',
      'use-isnan': 'error',
      'valid-typeof': 'error',
      '@typescript-eslint/ban-ts-comment': 'error',
      '@typescript-eslint/no-array-constructor': 'error',
      '@typescript-eslint/no-duplicate-enum-values': 'error',
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-extra-non-null-assertion': 'error',
      '@typescript-eslint/no-loss-of-precision': 'error',
      '@typescript-eslint/no-misused-new': 'error',
      '@typescript-eslint/no-namespace': 'error',
      '@typescript-eslint/no-non-null-asserted-optional-chain': 'error',
      '@typescript-eslint/no-this-alias': 'error',
      '@typescript-eslint/no-unnecessary-type-constraint': 'error',
      '@typescript-eslint/no-unsafe-declaration-merging': 'error',
      '@typescript-eslint/no-unused-vars': ['error', { caughtErrors: 'none' }],
      '@typescript-eslint/no-var-requires': 'error',
      '@typescript-eslint/prefer-as-const': 'error',
      '@typescript-eslint/triple-slash-reference': 'error',
      // ban-types was removed in v8; preserve its exact default banned types.
      '@typescript-eslint/no-restricted-types': ['error', {
        types: {
          "String": {
            "message": "Use string instead",
            "fixWith": "string"
          },
          "Boolean": {
            "message": "Use boolean instead",
            "fixWith": "boolean"
          },
          "Number": {
            "message": "Use number instead",
            "fixWith": "number"
          },
          "Symbol": {
            "message": "Use symbol instead",
            "fixWith": "symbol"
          },
          "BigInt": {
            "message": "Use bigint instead",
            "fixWith": "bigint"
          },
          "Function": {
            "message": "The `Function` type accepts any function-like value.\nIt provides no type safety when calling the function, which can be a common source of bugs.\nIt also accepts things like class declarations, which will throw at runtime as they will not be called with `new`.\nIf you are expecting the function to accept certain arguments, you should explicitly define the function shape."
          },
          "Object": {
            "message": "The `Object` type actually means \"any non-nullish value\", so it is marginally better than `unknown`.\n- If you want a type meaning \"any object\", you probably want `object` instead.\n- If you want a type meaning \"any value\", you probably want `unknown` instead.\n- If you really want a type meaning \"any non-nullish value\", you probably want `NonNullable<unknown>` instead.",
            "suggest": [
              "object",
              "unknown",
              "NonNullable<unknown>"
            ]
          },
          "{}": {
            "message": "`{}` actually means \"any non-nullish value\".\n- If you want a type meaning \"any object\", you probably want `object` instead.\n- If you want a type meaning \"any value\", you probably want `unknown` instead.\n- If you want a type meaning \"empty object\", you probably want `Record<string, never>` instead.\n- If you really want a type meaning \"any non-nullish value\", you probably want `NonNullable<unknown>` instead.",
            "suggest": [
              "object",
              "unknown",
              "Record<string, never>",
              "NonNullable<unknown>"
            ]
          }
        },
      }],
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
      // Add chosen shadcn/* rules here. Setup intentionally enables none.
    },
  },
]
