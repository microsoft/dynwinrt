// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { AIFeatureReadyState } from '#winapp/bindings/microsoft/windows/ai/AIFeatureReadyState'
import { AIFeatureReadyResultState } from '#winapp/bindings/microsoft/windows/ai/AIFeatureReadyResultState'
import type { AIFeatureReadyResult } from '#winapp/bindings/microsoft/windows/ai/AIFeatureReadyResult'
import type { RecognizedText } from '#winapp/bindings/microsoft/windows/ai/imaging/RecognizedText'
import type { TextRecognizer } from '#winapp/bindings/microsoft/windows/ai/imaging/TextRecognizer'
import { step } from './support.ts'

export interface ReadinessApi {
    getReadyState: typeof TextRecognizer.getReadyState
    ensureReadyAsync(signal?: AbortSignal): PromiseLike<
        Pick<AIFeatureReadyResult, 'status' | 'error' | 'extendedError' | 'errorDisplayText'>
    >
}

const AI_SETUP = 'Check supported NPU hardware, Windows updates, and the systemAIModels package capability (see README).'

export async function ensureModelReady(
    model: ReadinessApi,
    allowPreparation: boolean,
    report: (message: string) => void,
    signal?: AbortSignal,
): Promise<void> {
    const state = await step(`TextRecognizer.GetReadyState failed. ${AI_SETUP}`, () => model.getReadyState())
    switch (state) {
        case AIFeatureReadyState.Ready:
            report('TextRecognizer readiness: Ready (recognition has not run yet).')
            return
        case AIFeatureReadyState.NotSupportedOnCurrentSystem:
            throw new Error(`Windows AI OCR is not supported on this system. ${AI_SETUP}`)
        case AIFeatureReadyState.DisabledByUser:
            throw new Error('Windows AI OCR is disabled by the user. Review Windows AI settings; this sample will not change them.')
        case AIFeatureReadyState.NotReady:
            if (!allowPreparation) {
                throw new Error(
                    'The OCR model is NotReady. No download was requested. ' +
                    'Rerun with --ensure-ready only if you consent to Windows preparing/downloading the model.',
                )
            }
            break
        default:
            throw new Error(`Unrecognized TextRecognizer readiness state: ${state}. ${AI_SETUP}`)
    }
    report('Model preparation explicitly requested; Windows may download OCR model files.')
    const result = await step(`TextRecognizer.EnsureReadyAsync failed. ${AI_SETUP}`, () => model.ensureReadyAsync(signal))
    if (result === null) throw new Error('TextRecognizer.EnsureReadyAsync returned no readiness result.')
    if (result.status !== AIFeatureReadyResultState.Success) {
        const hex = (value: number) => `0x${(value >>> 0).toString(16).padStart(8, '0')}`
        throw new Error(
            `OCR model preparation did not succeed (status ${result.status}, error ${hex(result.error)}, ` +
            `extended error ${hex(result.extendedError)}). ${result.errorDisplayText} ${AI_SETUP}`,
        )
    }
    const after = await step(`Checking readiness after model preparation failed. ${AI_SETUP}`, () => model.getReadyState())
    if (after !== AIFeatureReadyState.Ready) {
        throw new Error(`The OCR model is still not Ready after preparation (state ${after}). ${AI_SETUP}`)
    }
    report('TextRecognizer readiness: Ready (recognition has not run yet).')
}

type TextLines = {
    readonly lines: readonly Pick<RecognizedText['lines'][number], 'text'>[]
}

export function recognizedLines(result: TextLines | null): string[] {
    if (result === null) throw new Error('TextRecognizer returned no RecognizedText result.')
    return result.lines.map((line) => {
        if (line === null) throw new Error('RecognizedText.Lines contained a null line.')
        return line.text
    })
}
