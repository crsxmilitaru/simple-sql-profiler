<!-- markdownlint-disable MD033 -->

<h1 align="center">🔎 Simple SQL Profiler</h1>

<p align="center">
  <strong>Fast, lightweight, and modern desktop SQL Profiler</strong>
</p>

<p align="center">
  <a href="https://crsxmilitaru.github.io/simple-sql-profiler/"><img src="https://img.shields.io/badge/-Website-0969da?style=flat" alt="Website"></a>
  <img src="https://img.shields.io/github/v/release/crsxmilitaru/simple-sql-profiler?label=Release&logo=github&logoColor=white" alt="GitHub Release">
  <a href="https://github.com/crsxmilitaru/simple-sql-profiler"><img src="https://img.shields.io/badge/GitHub-Repository-181717?logo=github&logoColor=white" alt="GitHub"></a>
  <a href="https://github.com/crsxmilitaru/simple-sql-profiler/stargazers"><img src="https://img.shields.io/github/stars/crsxmilitaru/simple-sql-profiler" alt="GitHub Stars"></a>
  <a href="https://github.com/crsxmilitaru/simple-sql-profiler/blob/main/LICENSE"><img src="https://img.shields.io/github/license/crsxmilitaru/simple-sql-profiler?style=flat" alt="License"></a>
  <a href="https://www.paypal.com/donate?hosted_button_id=MZQS5CZ68NGEW"><img src="https://img.shields.io/badge/Donate-PayPal-00457C?logo=paypal&logoColor=white" alt="Donate"></a>
</p>

---

<p align="center">
  <img src="preview.png" alt="Simple SQL Profiler Preview" style="width: 85%; max-width: 900px; height: auto; border-radius: 12px; border: 1px solid #30363d; box-shadow: 0 4px 10px rgba(0,0,0,0.25); vertical-align: middle;">
</p>

## ✨ Features

- **Real-time Capture**: Directly connects to your SQL Server via standard connection strings and captures running queries instantly.
- **Advanced Filtering**: Quickly locate specific queries, targeting specific databases, programs, or logins. Build complex conditions with the Advanced Filter Dialog to pinpoint specific events.
- **Deduplication**: Automatically deduplicates identical consecutive SQL statements to keep the live feed clean and easy to read.
- **Query Details**: View beautifully formatted SQL text accompanied by relevant performance metrics and timing statistics.
- **Smart Auto-scroll**: The query feed will automatically scroll to the newest entries, smoothly stopping when you interact to inspect a specific query.
- **Built-in Auto Updater**: Keep the app up to date with the integrated self-update mechanism leveraging Tauri's secure updater plugin.
- **Modern UI**: A sleek, custom dark mode interface optimized for long debugging sessions.

## 📖 Usage

1. **Launch** the application.
2. Enter your SQL Server connection details in the connection overlay. Use SQL Auth or Windows Auth as needed.
3. Click "Connect".
4. Hit the **Start Capture** button from the top toolbar to start recording SQL events.
5. Use the filter bar to perform basic text searches, or click the **Filter icon** to build complex conditional filters.
6. Click any row in the Feed to pull up the detailed Query inspection panel.

## 🚀 Built With

- **[Tauri](https://tauri.app/)** - Secure, lightweight, and incredibly fast desktop runtime.
- **[SolidJS](https://www.solidjs.com/)** - Reactive, high-performance UI framework.
- **[Tailwind CSS](https://tailwindcss.com/)** - Utility-first CSS framework for rapid UI design.
- **[FontAwesome](https://fontawesome.com/)** - Sleek vector icons.

## 🔒 Security & Privacy

- **Local-only communication:** The application connects directly to your SQL Server instance using secure local drivers. Extracted session data never leaves your machine.
- **No telemetry:** We do not track any usage, queries, connection strings, or interactions; there are zero hidden analytics or third-party tracking components.

---

<p align="center">
  <strong>💖 Support the Development</strong><br><br>
  If you find this application useful, consider buying me a coffee!<br><br>
  <a href="https://www.paypal.com/donate?hosted_button_id=MZQS5CZ68NGEW">
    <img src="https://www.paypalobjects.com/en_US/i/btn/btn_donateCC_LG.gif" alt="Donate with PayPal" />
  </a>
</p>

---

<p align="center">
  <strong>🙏 Thank you for using Simple SQL Profiler!</strong>
</p>
