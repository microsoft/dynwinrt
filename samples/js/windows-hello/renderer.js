// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const availability = document.querySelector('#availability')
const button = document.querySelector('#verify')
const result = document.querySelector('#result')

async function initialize() {
  try {
    const state = await window.windowsHello.checkAvailability()
    availability.textContent = `Availability: ${state.name}`
    button.disabled = state.name !== 'Available'
  } catch (error) {
    availability.textContent = error instanceof Error ? error.message : String(error)
  }
}

button.addEventListener('click', async () => {
  button.disabled = true
  result.textContent = 'Waiting for Windows Hello…'
  try {
    const verification = await window.windowsHello.verify()
    result.textContent = `Verification result: ${verification.name}`
  } catch (error) {
    result.textContent = error instanceof Error ? error.message : String(error)
  } finally {
    button.disabled = false
  }
})
void initialize()
void initialize()
