// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

export {}

const button = document.querySelector<HTMLButtonElement>('#share')
const text = document.querySelector<HTMLTextAreaElement>('#text')
const status = document.querySelector<HTMLDivElement>('#status')
const count = document.querySelector<HTMLSpanElement>('#count')

if (!button || !text || !status || !count) {
  throw new Error('The Share UI form is incomplete.')
}

const textInput = text
const countLabel = count

function updateCount(): void {
  const length = textInput.value.length
  countLabel.textContent = `${length} ${length === 1 ? 'character' : 'characters'}`
}

text.addEventListener('input', updateCount)
updateCount()

button.addEventListener('click', async () => {
  button.disabled = true
  status.dataset.state = 'pending'
  status.textContent = 'Opening the Windows Share UI…'

  try {
    await window.shareSample.show({
      title: 'Share from Electron',
      text: text.value
    })
    status.dataset.state = 'success'
    status.textContent = 'Share UI opened.'
  } catch (error) {
    status.dataset.state = 'error'
    status.textContent = error instanceof Error ? error.message : String(error)
  } finally {
    button.disabled = false
  }
})
