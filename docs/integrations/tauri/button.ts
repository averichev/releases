import { checkForUpdates } from './client';

// В HTML: <button id="check-updates">Проверить обновления</button>
//          <p id="update-status" role="status"></p>
const button = document.querySelector<HTMLButtonElement>('#check-updates');
const status = document.querySelector<HTMLElement>('#update-status');
button?.addEventListener('click', async () => {
  button.disabled = true;
  try {
    await checkForUpdates((message) => {
      if (status) status.textContent = message;
    });
  } finally { button.disabled = false; }
});
