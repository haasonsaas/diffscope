import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { Settings } from '../Settings'

const useConfigMock = vi.fn()
const mutateMock = vi.fn()

vi.mock('../../api/hooks', () => ({
  useConfig: () => useConfigMock(),
  useUpdateConfig: () => ({ mutate: mutateMock, isPending: false }),
  useAgentTools: () => ({ data: [] }),
}))

vi.mock('../../api/client', () => ({
  api: {
    getGhStatus: vi.fn(),
  },
}))

describe('Settings review context editors', () => {
  beforeEach(() => {
    useConfigMock.mockReset()
    mutateMock.mockReset()
    window.location.hash = ''
  })

  it('saves structured path instructions and custom context while preserving path metadata', async () => {
    const user = userEvent.setup()
    useConfigMock.mockReturnValue({
      data: {
        paths: {
          'src/auth/**': {
            review_instructions: 'Focus auth boundaries',
            focus: ['security'],
          },
        },
        custom_context: [{
          scope: 'auth',
          notes: ['Prefer tenant-safe queries'],
          files: ['docs/auth.md'],
        }],
      },
      isLoading: false,
    })

    render(<Settings />)

    await user.click(screen.getByRole('button', { name: 'Review' }))
    await user.click(screen.getByRole('button', { name: /REPOSITORY CONTEXT/i }))

    const pathInput = screen.getByDisplayValue('src/auth/**')
    const pathInstructions = screen.getByDisplayValue('Focus auth boundaries')
    const scopeInput = screen.getByDisplayValue('auth')
    const notesInput = screen.getByDisplayValue('Prefer tenant-safe queries')
    const filesInput = screen.getByDisplayValue('docs/auth.md')

    await user.clear(pathInput)
    await user.type(pathInput, 'src/auth/**')
    await user.clear(pathInstructions)
    await user.type(pathInstructions, 'Re-check auth and tenancy edges')
    await user.clear(scopeInput)
    await user.type(scopeInput, 'auth')
    await user.clear(notesInput)
    await user.type(notesInput, 'Keep audit logs intact')
    await user.clear(filesInput)
    await user.type(filesInput, 'docs/auth.md')

    await user.click(screen.getByRole('button', { name: 'Save' }))

    expect(mutateMock).toHaveBeenCalledWith(
      expect.objectContaining({
        paths: {
          'src/auth/**': {
            review_instructions: 'Re-check auth and tenancy edges',
            focus: ['security'],
          },
        },
        custom_context: [{
          scope: 'auth',
          notes: ['Keep audit logs intact'],
          files: ['docs/auth.md'],
        }],
      }),
      expect.any(Object),
    )
  }, 10_000)

  it('allows adding new path and custom context entries', async () => {
    const user = userEvent.setup()
    useConfigMock.mockReturnValue({ data: {}, isLoading: false })

    render(<Settings />)

    await user.click(screen.getByRole('button', { name: 'Review' }))
    await user.click(screen.getByRole('button', { name: /REPOSITORY CONTEXT/i }))
    await user.click(screen.getByRole('button', { name: /Add path/i }))
    await user.click(screen.getByRole('button', { name: /Add context/i }))

    await user.type(screen.getByPlaceholderText('tests/**'), 'tests/**')
    await user.type(screen.getByPlaceholderText('Focus on flaky tests, auth boundaries, and tenant isolation.'), 'Focus on flaky integration tests')
    await user.type(screen.getByPlaceholderText('auth|payments|docs/**'), 'tests')
    await user.type(screen.getByPlaceholderText(/Prefer tenant-safe queries/), 'Flaky tests are often async ordering issues')
    await user.type(screen.getByPlaceholderText(/docs\/auth\.md/), 'docs/testing.md')

    await user.click(screen.getByRole('button', { name: 'Save' }))

    expect(mutateMock).toHaveBeenCalledWith(
      expect.objectContaining({
        paths: {
          'tests/**': {
            review_instructions: 'Focus on flaky integration tests',
          },
        },
        custom_context: [{
          scope: 'tests',
          notes: ['Flaky tests are often async ordering issues'],
          files: ['docs/testing.md'],
        }],
      }),
      expect.any(Object),
    )
  }, 10_000)
})
