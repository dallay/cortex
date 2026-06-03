import { describe, expect, it } from 'vitest'
import { DEFAULT_SCOPES, SCOPES } from './scopes'

describe('SCOPES registry', () => {
  it('declares 5 scopes', () => {
    expect(SCOPES).toHaveLength(5)
  })

  it('declares scopes with unique values', () => {
    const values = SCOPES.map((s) => s.value)
    expect(new Set(values).size).toBe(values.length)
  })

  it('defaults every scope except admin to checked', () => {
    const defaults = SCOPES.filter((s) => s.defaultChecked).map((s) => s.value)
    expect(defaults).toContain('chat:read')
    expect(defaults).toContain('chat:write')
    expect(defaults).toContain('providers:read')
    expect(defaults).toContain('providers:write')
    expect(defaults).not.toContain('admin')
  })

  it('flags the admin scope as dangerous', () => {
    const admin = SCOPES.find((s) => s.value === 'admin')
    expect(admin).toBeDefined()
    expect(admin!.danger).toBe(true)
  })

  it('does not flag non-admin scopes as dangerous', () => {
    for (const scope of SCOPES) {
      if (scope.value === 'admin') continue
      expect(scope.danger).toBeFalsy()
    }
  })

  it('assigns every scope to a known group', () => {
    const groups = new Set(SCOPES.map((s) => s.group))
    for (const group of groups) {
      expect(['chat', 'providers', 'admin']).toContain(group)
    }
  })

  it('every scope has a non-empty label and description', () => {
    for (const scope of SCOPES) {
      expect(scope.label.length).toBeGreaterThan(0)
      expect(scope.description.length).toBeGreaterThan(0)
    }
  })
})

describe('DEFAULT_SCOPES', () => {
  it('contains 4 entries (all scopes except admin)', () => {
    expect(DEFAULT_SCOPES).toHaveLength(4)
  })

  it('does not include admin', () => {
    expect(DEFAULT_SCOPES).not.toContain('admin')
  })

  it('is a subset of SCOPES values', () => {
    const allValues = new Set(SCOPES.map((s) => s.value))
    for (const value of DEFAULT_SCOPES) {
      expect(allValues.has(value)).toBe(true)
    }
  })
})
