(() => {
  const root = document.documentElement;
  const system = matchMedia('(prefers-color-scheme: dark)');
  let preference;
  try { preference = localStorage.getItem('patchlook-theme'); } catch { /* Storage may be disabled. */ }
  if (!['light', 'dark'].includes(preference)) preference = null;

  const apply = () => {
    root.dataset.theme = preference || (system.matches ? 'dark' : 'light');
    const button = document.querySelector('.theme-button');
    if (button) {
      const label = root.dataset.theme === 'dark' ? button.dataset.lightLabel : button.dataset.darkLabel;
      button.setAttribute('aria-label', label);
      button.title = label;
    }
  };
  apply();
  system.addEventListener('change', apply);

  document.addEventListener('DOMContentLoaded', () => {
    const menus = [...document.querySelectorAll('.action-menu')];
    const close = (menu, restoreFocus = false) => {
      menu.open = false;
      if (restoreFocus) menu.querySelector('summary').focus();
    };
    const position = menu => {
      const panel = menu.querySelector('.action-popover');
      panel.style.removeProperty('--edge-shift');
      const bounds = panel.getBoundingClientRect();
      const gap = parseFloat(getComputedStyle(panel).getPropertyValue('--edge-gap'));
      const shift = Math.max(gap - bounds.left, Math.min(0, root.clientWidth - gap - bounds.right));
      panel.style.setProperty('--edge-shift', `${shift}px`);
    };
    const themeButton = document.querySelector('.theme-button');
    themeButton.hidden = false;
    apply();

    const platformTabs = [...document.querySelectorAll('[data-platform]')];
    const platformPanels = [...document.querySelectorAll('[data-platform-panel]')];
    const selectPlatform = platform => {
      platformTabs.forEach(tab => {
        const selected = tab.dataset.platform === platform;
        tab.setAttribute('aria-selected', String(selected));
        tab.tabIndex = selected ? 0 : -1;
      });
      platformPanels.forEach(panel => { panel.hidden = panel.dataset.platformPanel !== platform; });
    };
    platformPanels.forEach(panel => {
      panel.setAttribute('role', 'tabpanel');
      panel.setAttribute('aria-labelledby', `tab-${panel.dataset.platformPanel}`);
      panel.tabIndex = 0;
    });
    const platform = navigator.platform.toLowerCase();
    selectPlatform(platform.startsWith('win') ? 'windows'
      : platform.includes('linux') && !/android/i.test(navigator.userAgent) ? 'linux' : 'mac');
    document.querySelector('.download-box').classList.add('platform-ready');
    document.querySelector('.platform-tabs').hidden = false;
    platformTabs.forEach((tab, index) => {
      tab.addEventListener('click', () => selectPlatform(tab.dataset.platform));
      tab.addEventListener('keydown', event => {
        let next;
        if (event.key === 'ArrowRight') next = (index + 1) % platformTabs.length;
        else if (event.key === 'ArrowLeft') next = (index + platformTabs.length - 1) % platformTabs.length;
        else if (event.key === 'Home') next = 0;
        else if (event.key === 'End') next = platformTabs.length - 1;
        else return;
        event.preventDefault();
        selectPlatform(platformTabs[next].dataset.platform);
        platformTabs[next].focus();
      });
    });

    const downloadDialog = document.querySelector('#download-dialog');
    document.querySelector('[data-open-download]').addEventListener('click', event => {
      event.preventDefault();
      menus.forEach(menu => close(menu));
      root.classList.add('download-open');
      downloadDialog.showModal();
      platformTabs.find(tab => tab.getAttribute('aria-selected') === 'true').focus();
    });
    downloadDialog.querySelector('.close-download').addEventListener('click', () => downloadDialog.close());
    downloadDialog.addEventListener('close', () => root.classList.remove('download-open'));
    const outsideDialog = event => {
      const bounds = downloadDialog.getBoundingClientRect();
      return event.target === downloadDialog && (event.clientX < bounds.left || event.clientX > bounds.right
        || event.clientY < bounds.top || event.clientY > bounds.bottom);
    };
    let backdropPressed = false;
    downloadDialog.addEventListener('pointerdown', event => { backdropPressed = outsideDialog(event); });
    downloadDialog.addEventListener('click', event => {
      if (backdropPressed && outsideDialog(event)) downloadDialog.close();
      backdropPressed = false;
    });

    const copyButton = document.querySelector('.copy-command');
    if (navigator.clipboard?.writeText) {
      copyButton.hidden = false;
      const label = copyButton.getAttribute('aria-label');
      const copiedLabel = copyButton.dataset.copiedLabel;
      const feedback = document.querySelector('.command-feedback');
      let resetCopy;
      copyButton.addEventListener('click', async () => {
        clearTimeout(resetCopy);
        try {
          await navigator.clipboard.writeText(document.querySelector('#brew-command').textContent);
          copyButton.dataset.copied = 'true';
          copyButton.setAttribute('aria-label', copiedLabel);
          copyButton.title = copiedLabel;
          feedback.textContent = '';
        } catch {
          feedback.textContent = copyButton.dataset.error;
        }
        resetCopy = setTimeout(() => {
          delete copyButton.dataset.copied;
          copyButton.setAttribute('aria-label', label);
          copyButton.title = label;
        }, 2000);
      });
    }

    menus.forEach(menu => {
      menu.addEventListener('toggle', () => {
        if (menu.open) {
          menus.filter(other => other !== menu).forEach(other => close(other));
          position(menu);
        }
      });
    });
    window.addEventListener('resize', () => menus.filter(menu => menu.open).forEach(position));
    themeButton.addEventListener('click', () => {
      preference = root.dataset.theme === 'dark' ? 'light' : 'dark';
      try { localStorage.setItem('patchlook-theme', preference); } catch { /* Theme still works for this page. */ }
      apply();
    });
    for (const eventName of ['pointerdown', 'focusin']) {
      document.addEventListener(eventName, event => {
        menus.filter(menu => menu.open && !menu.contains(event.target)).forEach(menu => close(menu));
      });
    }
    document.addEventListener('keydown', event => {
      if (event.key !== 'Escape') return;
      const menu = menus.find(menu => menu.open);
      if (menu) {
        event.preventDefault();
        close(menu, true);
      }
    });
  });
})();
