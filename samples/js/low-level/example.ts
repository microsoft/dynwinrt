// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

export interface Resources {
    own<T extends { release(): void }>(value: T): T
    defer(label: string, action: () => void): void
}

export async function runExample(
    name: string,
    body: (resources: Resources) => string | Promise<string>,
    output: Pick<Console, 'log' | 'error'> = console,
): Promise<number> {
    const cleanup: { label: string; action: () => void }[] = []
    const errors: unknown[] = []
    let result: string | undefined
    const resources: Resources = {
        own(value) {
            cleanup.push({ label: 'WinRT value', action: () => value.release() })
            return value
        },
        defer(label, action) {
            cleanup.push({ label, action })
        },
    }
    try {
        result = await body(resources)
    } catch (error) {
        errors.push(error)
    }
    for (const { label, action } of cleanup.reverse()) {
        try {
            action()
        } catch (cause) {
            errors.push(new Error(`Cleanup failed: ${label}`, { cause }))
        }
    }
    if (errors.length > 0) {
        output.error(new AggregateError(errors, `${name} failed`))
        return 1
    }
    output.log(result)
    return 0
}
