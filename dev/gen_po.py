#!/usr/bin/env python3
"""Writes crates/app/translations/<lang>/LC_MESSAGES/fastpaste.po (bundled by build.rs). Edit the table, run, commit.
Slint's default gettext context is the component name, so msgctxt is `Main` (UI) or `Tr` (strings built in Rust)."""
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "app", "translations")
LANGS = ["pt_BR"]
# (context, msgid, [translation per LANGS])
T = [
    ("Main", "Search…", ["Pesquisar…"]),
    ("Main", "Copy some text or an image and it shows up here.", ["O que você copiar aparece aqui."]),
    ("Main", "No matches", ["Nenhum resultado"]),
    (
        "Main",
        "Enter paste · Shift+Enter options · PgUp/PgDn page · Alt+D delete",
        ["Enter colar · Shift+Enter opções · PgUp/PgDn página · Alt+D excluir"],
    ),
    ("Main", "Original", ["Original"]),
    ("Main", "Plain text", ["Texto simples"]),
    ("Main", "Trim", ["Aparar"]),
    ("Main", "UPPER CASE", ["MAIÚSCULAS"]),
    ("Main", "lower case", ["minúsculas"]),
    ("Main", "One line", ["Uma linha"]),
    ("Main", "OCR · Plain text", ["OCR · Texto simples"]),
    ("Main", "OCR · Trim", ["OCR · Aparar"]),
    ("Main", "OCR · UPPER CASE", ["OCR · MAIÚSCULAS"]),
    ("Main", "OCR · lower case", ["OCR · minúsculas"]),
    ("Main", "OCR · One line", ["OCR · Uma linha"]),
    ("Preferences", "FastPaste preferences", ["Preferências do FastPaste"]),
    ("Preferences", "System", ["Sistema"]),
    ("Preferences", "Shortcut", ["Atalho"]),
    ("Preferences", "Entries kept", ["Entradas guardadas"]),
    ("Preferences", "Language", ["Idioma"]),
    (
        "Preferences",
        "Also used to read text from images (OCR): this language first, then English, if their tesseract packs are installed.",
        ["Também usado para ler texto de imagens (OCR): este idioma primeiro, depois inglês, se os pacotes do tesseract estiverem instalados."],
    ),
    (
        "Preferences",
        "OCR is off: FastPaste cannot start the tesseract of your system. Run this once (tesseract must be installed on the system too):",
        ["OCR desativado: o FastPaste não consegue executar o tesseract do seu sistema. Execute isto uma vez (o tesseract também precisa estar instalado no sistema):"],
    ),
    (
        "Preferences",
        "OCR needs tesseract, which is not installed. Install it, with English, then see the help page.",
        ["O OCR precisa do tesseract, que não está instalado. Instale-o, com o inglês, e veja a página de ajuda."],
    ),
    (
        "Preferences",
        "OCR languages not installed: {}. Install them, then see the help page.",
        ["Idiomas do OCR não instalados: {}. Instale-os e veja a página de ajuda."],
    ),
    ("Preferences", "Help page", ["Página de ajuda"]),
    (
        "Preferences",
        "Use modifiers and one letter or digit, like ctrl+alt+v.",
        ["Use modificadores e uma letra ou dígito, como ctrl+alt+v."],
    ),
    (
        "Preferences",
        "That shortcut is already in use by another program.",
        ["Esse atalho já está em uso por outro programa."],
    ),
    (
        "Preferences",
        "Entries kept must be a number from 10 to 5000.",
        ["Entradas guardadas deve ser um número de 10 a 5000."],
    ),
    ("Preferences", "Cancel", ["Cancelar"]),
    ("Preferences", "Save", ["Salvar"]),
    ("Preferences", "Start at login", ["Iniciar com o sistema"]),
    ("Preferences", "Clipboard history", ["Histórico da área de transferência"]),
    ("Preferences", "Clear history…", ["Limpar histórico…"]),
    ("Preferences", "Clear history?", ["Limpar o histórico?"]),
    (
        "Preferences",
        "Every copied text and image will be deleted. This cannot be undone.",
        ["Todos os textos e imagens copiados serão apagados. Isso não pode ser desfeito."],
    ),
    ("Preferences", "Clear history", ["Limpar histórico"]),
    ("Preferences", "Quit FastPaste", ["Sair do FastPaste"]),
    ("Preferences", "About", ["Sobre"]),
    ("Preferences", "Close", ["Fechar"]),
    ("Preferences", "Version {}", ["Versão {}"]),
    ("Preferences", "Light and fast clipboard history for Linux.", ["Histórico da área de transferência leve e rápido para Linux."]),
    ("Preferences", "Author: {}", ["Autor: {}"]),
    ("Preferences", "License: {}", ["Licença: {}"]),
    ("Main", "Preferences", ["Preferências"]),
    ("Tr", "Preferences…", ["Preferências…"]),
    ("Tr", "Image {}×{} · {} KB", ["Imagem {}×{} · {} KB"]),
    ("Tr", "Open", ["Abrir"]),
    ("Tr", "Quit", ["Sair"]),
]

q = lambda s: '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'
for i, lang in enumerate(LANGS):
    d = f"{OUT}/{lang}/LC_MESSAGES"
    os.makedirs(d, exist_ok=True)
    with open(f"{d}/fastpaste.po", "w") as f:
        f.write(f'msgid ""\nmsgstr ""\n"Content-Type: text/plain; charset=UTF-8\\n"\n"Language: {lang}\\n"\n')
        for ctx, msgid, tr in T:
            assert len(tr) == len(LANGS) and tr[i].count("{}") == msgid.count("{}"), msgid
            f.write(f"\nmsgctxt {q(ctx)}\nmsgid {q(msgid)}\nmsgstr {q(tr[i])}\n")
