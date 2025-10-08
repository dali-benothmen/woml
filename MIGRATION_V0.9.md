# Migration Guide: Database Location Change (v0.9.0)

## What Changed?

In version 0.9.0, we moved the Cronflow database to a hidden directory to keep your project root clean.

### Before (< v0.9.0):

```
my-project/
  ├── cronflow.db          ❌ Visible in root
  ├── node_modules/
  ├── package.json
  └── workflow.js
```

### After (>= v0.9.0):

```
my-project/
  ├── .cronflow/           ✅ Hidden directory
  │   └── data.db
  ├── node_modules/
  ├── package.json
  └── workflow.js
```

## Do You Need to Migrate?

**For most users: NO!**

- If you're starting fresh, everything works automatically
- The old `cronflow.db` will be ignored if it exists
- New installations will use `.cronflow/data.db` automatically

## Migration Options

### Option 1: Start Fresh (Recommended for Development)

Simply delete the old database and let Cronflow create a new one:

```bash
rm cronflow.db
```

Next time you run your workflow, Cronflow will automatically:

1. Create `.cronflow/` directory
2. Create `data.db` inside it
3. Initialize the schema

**Data Loss**: Yes (workflow execution history is lost)  
**Recommended for**: Development, testing, or if you don't need historical data

---

### Option 2: Migrate Existing Data

If you need to keep your workflow execution history:

```bash
# Create the new directory
mkdir -p .cronflow

# Move the database
mv cronflow.db .cronflow/data.db
```

**Data Loss**: No (all history preserved)  
**Recommended for**: Production environments with important execution history

---

### Option 3: Custom Database Path

If you prefer to keep the database in a specific location, you can set the path via environment variable:

```bash
export CRONFLOW_DB_PATH="./my-custom-location.db"
```

Or in your code (though this requires modifying Rust core):

```javascript
// Note: This is handled by the Rust core
// The default is now .cronflow/data.db
cronflow.start();
```

**Data Loss**: No  
**Recommended for**: Specific deployment requirements

---

## Verifying the Migration

After migrating, verify the new location:

```bash
# List hidden directories
ls -la

# You should see:
# drwxr-xr-x   .cronflow/

# Check the database exists
ls -la .cronflow/

# You should see:
# -rw-r--r--   data.db
```

## Updating Your .gitignore

If you have `cronflow.db` in your `.gitignore`, you can update it:

```gitignore
# Old (still works but not needed)
cronflow.db

# New (more explicit)
.cronflow/

# Or use wildcards (catches all .db files)
*.db
```

## Backward Compatibility

### Environment Variable Override

The new version still respects the `CRONFLOW_DB_PATH` environment variable:

```bash
# Use old location
export CRONFLOW_DB_PATH="cronflow.db"

# Use custom location
export CRONFLOW_DB_PATH="/var/lib/cronflow/data.db"
```

### No Code Changes Required

Your application code doesn't need any changes. The database location is handled internally by the Rust core.

## Rollback

If you need to rollback to the old location:

```bash
# Move database back
mv .cronflow/data.db cronflow.db

# Remove the directory
rmdir .cronflow

# Set environment variable to use old location
export CRONFLOW_DB_PATH="cronflow.db"
```

## Troubleshooting

### "Can't see .cronflow directory"

Hidden directories (starting with `.`) are not shown by default:

```bash
# Show hidden files and directories
ls -la

# Or in file managers, enable "Show hidden files"
```

### "Permission denied when creating .cronflow"

Ensure your process has write permissions:

```bash
# Check permissions
ls -ld .

# If needed, fix permissions
chmod u+w .
```

### "Database locked" errors

If you're getting database locked errors after migrating:

1. Stop all Cronflow processes
2. Move the database to the new location
3. Restart Cronflow

```bash
# Kill any running Cronflow processes
pkill -f cronflow

# Move database
mkdir -p .cronflow
mv cronflow.db .cronflow/data.db

# Restart your application
node workflow.js
```

## Benefits of This Change

✅ **Cleaner Project Root**: Hidden directory keeps your project organized  
✅ **Standard Practice**: Follows conventions like `.git/`, `.next/`, `.cache/`  
✅ **Better Gitignore**: Easier to exclude from version control  
✅ **Scalability**: Room for additional Cronflow data (logs, cache, etc.)  
✅ **Professionalism**: More polished for open-source projects

## Questions?

If you have any questions or issues with the migration:

1. Check the [Setup Guide](./SETUP_GUIDE.md)
2. Review the [troubleshooting section](./SETUP_GUIDE.md#common-issues)
3. Open a [GitHub Discussion](https://github.com/dali-benothmen/cronflow/discussions)
4. Report issues on [GitHub Issues](https://github.com/dali-benothmen/cronflow/issues)

---

**Version**: 0.9.0  
**Date**: January 2025  
**Breaking Change**: Minor (easy migration)
