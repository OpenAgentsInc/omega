---
title: Finding and Navigating Code - Zed
description: Navigate your codebase in Zed with file finder, project search, go to definition, symbol search, and the command palette.
---

# Finding & Navigating

Zed provides several ways to move around your codebase quickly. Here's an overview of the main navigation tools.

## Command Palette

The Command Palette ({#kb command_palette::Toggle}) is your gateway to almost everything in Zed. Type a few characters to filter commands, then press Enter to execute.

[Learn more about the Command Palette →](./command-palette.md)

## Project Panel

The Project Panel ({#kb project_panel::ToggleFocus}) shows a tree view of your workspace's files and directories. Browse, create, rename, move, and delete files without leaving the editor. It also surfaces git status and diagnostics at a glance.

[Learn more about the Project Panel →](./project-panel.md)

## File Finder

Open any file in your project with {#kb file_finder::Toggle}. Type part of the filename or path to narrow results.

## Text Finder

Quickly find any string in your project and open the file with {#kb project_search::OpenTextFinder}. Changed your mind and want a more detailed search with extra filters? Move to the project search using the button in the Actions menu in the right bottom corner.

## Project Search

Search across all files with {#kb pane::DeploySearch}. Type the query in the search field, then press Enter to run the search.

Results appear in a [multibuffer](./multibuffers.md), letting you edit matches in place.

## Go to Definition

Jump to where a symbol is defined with {#kb editor::GoToDefinition} (or `Cmd+Click` / `Ctrl+Click`). If there are multiple definitions, they open in a multibuffer.

## Quick Reference

| Task               | Keybinding                           |
| ------------------ | ------------------------------------ |
| Command Palette    | {#kb command_palette::Toggle}        |
| Open file          | {#kb file_finder::Toggle}            |
| Project search     | {#kb pane::DeploySearch}             |
| Text search picker | {#kb project_search::OpenTextFinder} |
| Go to definition   | {#kb editor::GoToDefinition}         |
| Find references    | {#kb editor::FindAllReferences}      |
| Project Panel      | {#kb project_panel::ToggleFocus}     |
