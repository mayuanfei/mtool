const t = localStorage.getItem('mtool_theme');
document.documentElement.setAttribute('data-theme', t === 'light' ? 'light' : 'dark');
