// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

function attachValidation(app, window) {
  window.webContents.once("did-finish-load", async () => {
    try {
      const result = await window.webContents.executeJavaScript(`
        (async () => {
          const waitFor = async (predicate, description) => {
            const deadline = Date.now() + 300000;
            while (!predicate()) {
              if (document.querySelector('#model-status-text')?.textContent === 'Unavailable') {
                throw new Error(document.querySelector('#runtime-detail').textContent);
              }
              if (Date.now() >= deadline) throw new Error('Timed out: ' + description);
              await new Promise((resolve) => setTimeout(resolve, 25));
            }
          };
          const input = () => document.querySelector('#prompt');
          const metrics = () => document.querySelector('#metrics').textContent;
          const response = () => document.querySelector('.message:last-child .message-text');
          const send = async (prompt) => {
            Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')
              .set.call(input(), prompt);
            input().dispatchEvent(new Event('input', { bubbles: true }));
            await waitFor(() => !document.querySelector('#send').disabled, 'enabled Send button');
            const previousCount = document.querySelectorAll('.message').length;
            document.querySelector('#prompt-form').requestSubmit();
            await waitFor(
              () => document.querySelectorAll('.message').length === previousCount + 2,
              'submitted React messages',
            );
          };
          const reset = async () => {
            document.querySelector('#new-chat').click();
            await waitFor(
              () => metrics() === 'Conversation context reset' && !input().disabled,
              'conversation reset',
            );
          };

          await waitFor(() => input() && !input().disabled, 'React model initialization');
          const progress = [];
          const observer = new MutationObserver(() => {
            const element = response();
            if (element?.classList.contains('streaming') &&
                element.textContent && !element.textContent.startsWith('Thinking')) {
              progress.push(element.textContent);
            }
          });
          observer.observe(document.querySelector('#messages'), {
            childList: true, characterData: true, subtree: true,
          });
          const expected = 'DYNWINRT_ELECTRON_OK';
          let answer;
          try {
            await send('Reply with exactly ' + expected + ' and nothing else.');
            await waitFor(
              () => !input().disabled && !document.querySelector('.streaming'),
              'React-rendered response',
            );
            answer = response().textContent.trim();
          } finally {
            observer.disconnect();
          }
          const completedMetrics = metrics();

          await reset();
          const resetClearedMessages = document.querySelectorAll('.message').length === 1;
          await send('Write 100 numbered sentences describing different imaginary landscapes.');
          await waitFor(
            () => document.querySelector('.streaming') &&
              response().textContent && !response().textContent.startsWith('Thinking'),
            'streaming before Stop',
          );
          document.querySelector('#stop').click();
          await waitFor(
            () => !input().disabled && !document.querySelector('#stop'),
            'cancellation completion',
          );
          const canceled = response().classList.contains('canceled') &&
            metrics().startsWith('Stopped after');
          await reset();

          return {
            answer,
            expected,
            progressUpdates: progress.length,
            progressMatches: progress.length > 0 &&
              progress.every((partial) => expected.startsWith(partial)),
            completed: completedMetrics.startsWith('Complete'),
            resetClearedMessages,
            canceled,
          };
        })()
      `);
      const layout = await window.webContents.executeJavaScript(`
        (() => {
          const root = document.documentElement;
          const messages = document.querySelector('#messages');
          const required = [
            '.hero',
            '.chat-toolbar',
            '#messages',
            '.suggestions',
            '#prompt-form',
            'footer',
          ].map((selector) => document.querySelector(selector));
          const allSectionsVisible = required.every((element) => {
            if (!element) return false;
            const box = element.getBoundingClientRect();
            return box.width > 0 && box.height > 0 &&
              box.top >= 0 && box.bottom <= innerHeight &&
              box.left >= 0 && box.right <= innerWidth;
          });
          return {
            viewport: { width: innerWidth, height: innerHeight },
            documentFits:
              root.scrollWidth <= root.clientWidth &&
              root.scrollHeight <= root.clientHeight,
            allSectionsVisible,
            messagesScrollbarHidden:
              getComputedStyle(messages).scrollbarWidth === 'none',
          };
        })()
      `);
      const valid =
        result.answer === result.expected &&
        result.progressMatches &&
        result.completed &&
        result.resetClearedMessages &&
        result.canceled &&
        layout.documentFits &&
        layout.allSectionsVisible &&
        layout.messagesScrollbarHidden;
      console.log(JSON.stringify({ valid, layout, ...result }, null, 2));
      window.destroy();
      app.exit(valid ? 0 : 1);
    } catch (error) {
      console.error(error);
      window.destroy();
      app.exit(1);
    }
  });
}

module.exports = { attachValidation };
