# Security and Performance Fixes Template
# Example fixes for common vulnerabilities in Python web applications
# 
# CRITICAL FIXES DEMONSTRATED:

"""
1. SQL Injection Fix in organizations.py
"""

# BEFORE (VULNERABLE):
# if query:
#     search_term = f"%{query}%"
#     base_query = base_query.where(
#         (User.name.ilike(search_term)) | (User.email.ilike(search_term))
#     )

# AFTER (SECURE):
from sqlalchemy import text

if query:
    # Use parameterized queries to prevent SQL injection
    base_query = base_query.where(
        text("(users.name ILIKE :search OR users.email ILIKE :search)")
    ).params(search=f"%{query}%")

"""
2. Input Validation Enhancement
"""

from pydantic import EmailStr, Field, validator
from typing import List

class BulkInvitationCreateRequest(BaseModel):
    """Enhanced request model with proper validation."""
    
    emails: List[EmailStr] = Field(..., min_items=1, max_items=50, description="Maximum 50 emails per bulk invite")
    firstName: str = Field("", max_length=100)
    lastName: str = Field("", max_length=100) 
    role: str = Field(..., regex="^(owner|admin|member|auditor)$")
    expires_in_days: int = Field(7, ge=1, le=30)
    
    @validator('emails')
    def validate_unique_emails(cls, v):
        """Ensure no duplicate emails in the list."""
        if len(v) != len(set(v)):
            raise ValueError("Duplicate emails are not allowed")
        return v
    
    @validator('role')
    def validate_role(cls, v):
        """Validate role against allowed values."""
        valid_roles = ["owner", "admin", "member", "auditor"]
        if v not in valid_roles:
            raise ValueError(f"Role must be one of: {', '.join(valid_roles)}")
        return v

"""
3. Transaction Management for Bulk Operations
"""

async def create_bulk_organization_invitations(
    org_id: str,
    invitation_data: BulkInvitationCreateRequest,
    request: Request,
    user_context: Dict[str, Any] = Depends(require_authenticated_user),
    db: AsyncSession = Depends(get_db),
) -> Dict[str, Any]:
    """Create bulk organization invitations with proper transaction management."""
    
    # Verify organization access
    await require_organization_access(request, org_id, db)
    
    try:
        # Use transaction for atomic operations
        async with db.begin():
            successful_invites = []
            failed_invites = []
            
            # Check if organization exists
            org_result = await db.execute(
                select(Organization).where(Organization.id == org_id)
            )
            organization = org_result.scalar_one_or_none()
            
            if not organization:
                raise HTTPException(status_code=404, detail="Organization not found")
            
            # Process each email
            for email in invitation_data.emails:
                try:
                    # Check if user already exists
                    user_result = await db.execute(
                        select(User).where(User.email == email)
                    )
                    existing_user = user_result.scalar_one_or_none()
                    
                    if existing_user:
                        # Check if already a member
                        membership_result = await db.execute(
                            select(UserOrganization)
                            .where(UserOrganization.user_id == existing_user.id)
                            .where(UserOrganization.organization_id == org_id)
                        )
                        if membership_result.scalar_one_or_none():
                            failed_invites.append({
                                "email": email,
                                "reason": "User is already a member"
                            })
                            continue
                    
                    # Check for existing invitation
                    invitation_result = await db.execute(
                        select(OrganizationInvitation)
                        .where(OrganizationInvitation.email == email)
                        .where(OrganizationInvitation.organization_id == org_id)
                        .where(OrganizationInvitation.status == "pending")
                    )
                    existing_invitation = invitation_result.scalar_one_or_none()
                    
                    if existing_invitation:
                        failed_invites.append({
                            "email": email,
                            "reason": "Invitation already pending"
                        })
                        continue
                    
                    # Create new invitation
                    invitation = OrganizationInvitation(
                        organization_id=org_id,
                        email=email,
                        role=invitation_data.role,
                        invited_by=user_context["user_id"],
                        expires_at=datetime.utcnow() + timedelta(days=invitation_data.expires_in_days),
                        status="pending"
                    )
                    
                    db.add(invitation)
                    await db.flush()  # Get the ID without committing
                    
                    successful_invites.append({
                        "email": email,
                        "invitation_id": invitation.id,
                        "role": invitation_data.role
                    })
                    
                    # TODO: Send invitation email (implement async email service)
                    
                except Exception as e:
                    failed_invites.append({
                        "email": email,
                        "reason": f"Processing error: {str(e)}"
                    })
            
            # If no successful invites, rollback and return error
            if not successful_invites:
                raise HTTPException(
                    status_code=400,
                    detail="No invitations could be processed"
                )
            
            # Commit the transaction
            await db.commit()
            
            return {
                "success": True,
                "data": {
                    "successful_invites": successful_invites,
                    "failed_invites": failed_invites,
                    "summary": {
                        "total_attempted": len(invitation_data.emails),
                        "successful": len(successful_invites),
                        "failed": len(failed_invites)
                    }
                }
            }
            
    except HTTPException:
        raise
    except Exception as e:
        await db.rollback()
        logger.error(f"Bulk invitation failed: {str(e)}")
        raise HTTPException(
            status_code=500,
            detail="Failed to process bulk invitations"
        )

"""
4. Rate Limiting Implementation
"""

from slowapi import Limiter, _rate_limit_exceeded_handler
from slowapi.util import get_remote_address
from slowapi.errors import RateLimitExceeded

# Add to main app setup
limiter = Limiter(key_func=get_remote_address)
app.state.limiter = limiter
app.add_exception_handler(RateLimitExceeded, _rate_limit_exceeded_handler)

# Apply to sensitive endpoints
@router.post("/{org_id}/members/invite-bulk")
@limiter.limit("5/minute")  # Max 5 bulk invites per minute
async def create_bulk_organization_invitations(
    request: Request,  # Required for limiter
    org_id: str,
    invitation_data: BulkInvitationCreateRequest,
    # ... rest of params
):
    # ... implementation

"""
5. Performance Optimization with Eager Loading
"""

async def get_organization_members(
    org_id: str,
    page: int = Query(1, ge=1),
    limit: int = Query(50, ge=1, le=100),
    query: Optional[str] = Query(None),
    sortBy: Optional[str] = Query(None),
    sortOrder: str = Query("asc"),
    user_context: Dict[str, Any] = Depends(require_authenticated_user),
    db: AsyncSession = Depends(get_db),
) -> Dict[str, Any]:
    """Optimized member listing with proper eager loading."""
    
    try:
        # Single query with eager loading to prevent N+1
        base_query = (
            select(UserOrganization)
            .options(
                selectinload(UserOrganization.user),  # Eager load users
                selectinload(UserOrganization.organization)  # Eager load org if needed
            )
            .join(User, UserOrganization.user_id == User.id)
            .where(UserOrganization.organization_id == org_id)
            .where(User.deleted_at.is_(None))
        )
        
        # Add search with safe parameterized query
        if query:
            search_param = f"%{query}%"
            base_query = base_query.where(
                text("(users.name ILIKE :search OR users.email ILIKE :search)")
            ).params(search=search_param)
        
        # Get total count efficiently
        count_query = (
            select(func.count(UserOrganization.id))
            .select_from(UserOrganization)
            .join(User, UserOrganization.user_id == User.id)
            .where(UserOrganization.organization_id == org_id)
            .where(User.deleted_at.is_(None))
        )
        
        if query:
            search_param = f"%{query}%"
            count_query = count_query.where(
                text("(users.name ILIKE :search OR users.email ILIKE :search)")
            ).params(search=search_param)
        
        # Execute count and main query concurrently
        count_result, members_result = await asyncio.gather(
            db.execute(count_query),
            db.execute(
                base_query
                .order_by(UserOrganization.created_at.desc())
                .offset((page - 1) * limit)
                .limit(limit)
            )
        )
        
        total = count_result.scalar() or 0
        memberships = members_result.scalars().all()
        
        # Process results (no additional queries needed due to eager loading)
        members_data = []
        for membership in memberships:
            # Data is already loaded, no additional queries
            full_name = membership.user.name or ""
            name_parts = full_name.split(" ", 1) if full_name else ["", ""]
            
            members_data.append({
                "id": membership.id,
                "email": membership.user.email or "",
                "fullName": full_name,
                "firstName": name_parts[0] if name_parts else "",
                "lastName": name_parts[1] if len(name_parts) > 1 else "",
                "role": {
                    "name": membership.role,
                    "slug": membership.role.lower().replace(" ", "_"),
                },
                "joinedAt": membership.created_at.isoformat() if membership.created_at else None,
                "lastActive": membership.user.last_login_at.isoformat() if membership.user.last_login_at else None,
                "status": "active" if membership.user.is_active else "inactive",
                "userId": membership.user.id,
            })
        
        return {
            "success": True,
            "data": {
                "members": members_data,
                "pagination": {
                    "total": total,
                    "page": page,
                    "limit": limit,
                    "pages": (total + limit - 1) // limit,
                }
            }
        }
        
    except Exception as e:
        logger.error(f"Error fetching organization members: {str(e)}")
        raise HTTPException(
            status_code=500,
            detail="Failed to fetch organization members"
        )

"""
6. Enhanced Error Handling with Structured Responses
"""

from enum import Enum
from pydantic import BaseModel

class ErrorCode(str, Enum):
    """Standardized error codes for API responses."""
    MEMBER_NOT_FOUND = "MEMBER_NOT_FOUND"
    INVALID_ROLE = "INVALID_ROLE"
    LAST_ADMIN_REMOVAL = "LAST_ADMIN_REMOVAL"
    ORGANIZATION_NOT_FOUND = "ORGANIZATION_NOT_FOUND"
    DUPLICATE_EMAIL = "DUPLICATE_EMAIL"
    VALIDATION_ERROR = "VALIDATION_ERROR"
    RATE_LIMIT_EXCEEDED = "RATE_LIMIT_EXCEEDED"
    UNAUTHORIZED = "UNAUTHORIZED"

class APIError(BaseModel):
    """Structured error response model."""
    code: ErrorCode
    message: str
    details: Optional[Dict[str, Any]] = None
    field: Optional[str] = None  # For validation errors

class APIResponse(BaseModel):
    """Standardized API response model."""
    success: bool
    data: Optional[Any] = None
    error: Optional[APIError] = None
    meta: Optional[Dict[str, Any]] = None

def create_error_response(
    code: ErrorCode,
    message: str,
    details: Optional[Dict[str, Any]] = None,
    field: Optional[str] = None
) -> Dict[str, Any]:
    """Create standardized error response."""
    return {
        "success": False,
        "error": {
            "code": code.value,
            "message": message,
            "details": details,
            "field": field
        }
    }

# Usage in endpoints:
if not membership:
    return create_error_response(
        ErrorCode.MEMBER_NOT_FOUND,
        "Member not found in this organization",
        details={"member_id": member_id, "org_id": org_id}
    )