import { afterEach, describe, expect, it, vi } from 'vitest'
import { Api, ApiError, assertSafeNumbers, parseConfiguration } from '../src/api'
const ok = (value: unknown) => new Response(JSON.stringify(value), { status: 200 })
afterEach(() => vi.unstubAllGlobals())
describe('same-origin management client', () => {
  it('keeps credentials in memory and attaches CSRF to every management call and logout', async () => {
    const fetcher = vi.fn().mockResolvedValueOnce(ok({ csrf_token: 'csrf', expires_in_secs: 600 })).mockResolvedValueOnce(ok({ result: [] })).mockResolvedValueOnce(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetcher)
    const api = new Api()
    await api.session('temporary-token')
    await api.rpc('plugins.list')
    expect(fetcher.mock.calls[1][0]).toBe('/api/v1/manage')
    expect(fetcher.mock.calls[1][1]).toMatchObject({ mode: 'same-origin', credentials: 'same-origin', headers: { 'X-CSRF-Token': 'csrf' }, body: JSON.stringify({ method: 'plugins.list', payload: {} }) })
    await api.logout()
    expect(fetcher.mock.calls[2][1].headers['X-CSRF-Token']).toBe('csrf')
    expect(api.csrf).toBe('')
  })
  it('ignores a delayed old 401 after a new session is established', async () => {
    let finish: (response: Response) => void = () => {}
    vi.stubGlobal('fetch', vi.fn().mockImplementationOnce(() => new Promise(resolve => { finish = resolve })).mockResolvedValueOnce(ok({ csrf_token: 'new-csrf', expires_in_secs: 600 })))
    const api = new Api(); api.csrf = 'old-csrf'; api.onExpired = vi.fn()
    const previous = api.rpc('plugins.list').catch(e => e)
    await api.session('new-token')
    finish(new Response(JSON.stringify({ error: { code: 'Unauthorized', message: 'expired' } }), { status: 401 }))
    expect(await previous).toMatchObject({ code: 'StaleSession' })
    expect(api.csrf).toBe('new-csrf')
    expect(api.onExpired).not.toHaveBeenCalled()
  })
  it('drops delayed successful data from an old session', async () => {
    let finish: (response: Response) => void = () => {}
    vi.stubGlobal('fetch', () => new Promise(resolve => { finish = resolve }))
    const api = new Api(); const request = api.rpc('plugins.list').catch(e => e)
    api.invalidate(); finish(ok({ result: ['old'] }))
    expect(await request).toMatchObject({ code: 'StaleSession' })
  })
  it('expires the current session on 401 and never retries writes', async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: { code: 'Unauthorized', message: 'expired' } }), { status: 401 }))
    vi.stubGlobal('fetch', fetcher)
    const api = new Api(); api.csrf = 'csrf'; api.onExpired = vi.fn()
    await expect(api.rpc('config.update')).rejects.toMatchObject({ status: 401 })
    expect(api.onExpired).toHaveBeenCalledOnce(); expect(api.csrf).toBe(''); expect(fetcher).toHaveBeenCalledOnce()
  })
})
describe('configuration preservation', () => {
  it('roundtrips nested secret references without resolving or replacing them', () => {
    const values = { auth: { $secret: 'env:TWINTAIL_TOKEN' }, nested: [{ reference: { $secret: 'env:OTHER' } }] }
    expect(parseConfiguration(JSON.stringify(values))).toEqual(values)
  })
  it.each(['{"token":"***"}', '{"nested":["<redacted>"]}', '{"token":"[REDACTED]"}', '[]', '{', '{"value":9007199254740993}', '{"value":1e999}'])('rejects unsafe JSON without modifying input: %s', text => {
    expect(() => parseConfiguration(text)).toThrow(ApiError)
  })
  it('rejects unsafe revisions while allowing ordinary fractional numbers', () => {
    expect(() => assertSafeNumbers({ revision: Number.MAX_SAFE_INTEGER + 1 })).toThrow(/CLI/)
    expect(parseConfiguration('{"value":0.25}')).toEqual({ value: 0.25 })
  })
})
