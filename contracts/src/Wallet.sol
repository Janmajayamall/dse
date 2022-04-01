// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import "./interfaces/IERC20.sol";
import "./libraries/Transfers.sol";

contract Wallet {

    using Transfers for IERC20;

    mapping(bytes32 => bool) claimed;
    mapping(bytes32 => uint256) burnBufferTime;
    mapping(bytes32 => uint256) burnBufferAmount;
    mapping(bytes32 => bytes32) burnBufferIBlob;
    mapping(bytes32 => address) burnBufferIAddress;

    // Wallet's on going epoch
    uint public currentEpoch;
    // Epoch's end time 
    uint public epochExpiresBy;
    // Wallet's security deposit
    uint public securityDeposit;
    // ERC20 token's address used as currency by wallet
    address public currency;
    // Owner of the wallet
    address public owner;

    // Duration of a epoch
    uint immutable public epochDuration;
    // Value in currency per index
    uint immutable public indexValue;
    // Buffer time for burn
    uint immutable public burnBuffer;
    
    // Buffer time added to burn buffer time
    // of commitments in which owner isn't 
    // the invalidating address
    // This is needed to prevent attack
    // by iAddress when they prevent their funds 
    // from burning by producing signature after
    // funds of their counter party have been 
    // burnt, since buffer time is equal irrespective
    // of who is at the advantage of producing invaldating
    // signature.
    uint constant public invDisadvantageBuffer;

    error BalanceError();
    error NotOwner();
    error EpochOngoing();
     
    constructor(
        uint _epochDuration,
        uint _indexValue,
        address _owner,
        address _currency,
        uint _bufferTime
    ) {
        epochDuration = _epochDuration;
        indexValue = _indexValue;
        owner = _owner;
        currency = _currency;
        bufferTime = _bufferTime;

        currentEpoch = 1;
        epochExpiresBy = block.timestamp + _epochDuration;
    }

    function getBalance(
        address token
    ) internal view returns (uint256 balance) {
        (bool success, bytes memory data) = token.staticcall(
            abi.encodeWithSelector(IERC20.balanceOf.selector, address(this))
        );
        if (!success || data.length != 32) revert BalanceError();
        balance = abi.decode(data, (uint256));
    }

    function isValidCommitment(
        uint256 index,
        uint256 epoch
    ) public view returns (bool isValid) {
        // zero is not a valid index
        if (index == 0) return false;
        // not current epoch or epoch expired
        if (
            epoch != currentEpoch ||
            block.timestamp >= epochExpiresBy
        ) return false;
        
        // Wallet's active balance = Total Balance - Security Deposit
        if (index <= ((getBalance(currency) - securityDeposit) / indexValue)){
            return true;
        }
        return false;
    }

    /// Deposit amount as security deposit for
    /// the wallet. 
    function deposit() public {
        uint256 _securityDeposit = securityDeposit;
        uint256 amount = getBalance(currency) - _securityDeposit;
        securityDeposit = amount + _securityDeposit;
        // TODO emit event
    }

    /// Owner withdraws security deposit +
    /// not redeemed balance in the wallet
    function withdraw() public {
        if (msg.sender != owner) revert NotOwner();

        // cannot withdraw during an active epoch
        if (block.timestamp < epochDuration) revert EpochOngoing();

        address token = currency;
        IERC20(token).safeTransfer(msg.sender, getBalance(token));

        securityDeposit = 0;
    }

    /// Renews wallet for next epoch
    function renew() public {
        if (block.timestamp < epochDuration) revert EpochOngoing();

        currentEpoch += 1;
        epochExpiresBy = block.timestamp + epochExpiresBy;

        // TODO emit event
    }

    /// Redeem time-locked commitments of type 2 
    function redeemdType2(
        uint32[] calldata indexes,
        uint32[] calldata u,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s,
        uint8[] calldata vi,
        bytes32[] calldata ri,
        bytes32[] calldata si,
        address to,
    ) public {
        // sanity checks
        if (block.timestamp > epochExpiresBy) revert();
        if (indexes.length != v.length) revert();
        if (
            v.length != u.length ||
            v.length != r.length || 
            v.length != s.length ||
            v.length != vi.length ||
            v.length != ri.length ||
            v.length != si.length ||
            v.length != vi.length ||
        ) revert();

        uint256 _indexValue = indexValue;
        uint256 _currentEpoch = currentEpoch;
        address _owner = owner;

        // total amount to be redeemed
        uint256 totalAmount;

        for (uint256 i = 0; i < indexes.length; i++) {
            bytes32 commit = keccak256(abi.encodePacked(
                uint256(indexes[i]), 
                uint256(epoch), 
                uint256(u[i]), 
                uint256(2), 
                _owner, 
                to
            ));

            if (claimed[commit] == true) revert();

            // checks that owner signed the commitment
            if (ecrecover(commit, v[i], r[i], s[i]) != _owner) revert();

            // check that owner signed the invalidating message
            bytes32 iCommit = keccak256(abi.encodePacked(u[i]));
            if (ecrecover(iCommit, vi[i], ri[i], si[i]) != _owner) revert();

            totalAmount += _indexValue

            claimed[commit] = true;
        }

        IERC20(currency).transfer(to, totalAmount);

        // TODO emit claimType2
    }

    /// Burn commitments with no invalidating
    /// signatures
    /// Note that arrays are split into two 
    /// parts defined by splitIndex.
    /// First part is for type 1 commits
    /// Second part is for type 2 commits
    function burnCommits(
        uint32[] calldata indexes,
        uint32[] calldata u,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s,
        address type1IAddress,
        address type2ToAddress,
        uint256 splitIndex
    ) public {
        uint _bufferTime = bufferTime;

        // Can't call burn after epochExpiry - bufferTime - invDisadvantageBuffer.
        if (block.timestamp >= epochExpiresBy - (_bufferTime + invDisadvantageBuffer)) revert();

        // sanity checks
        if (indexes.length != v.length) revert();
        if (
            v.length != r.length || 
            v.length != s.length ||
            v.length != u.length
        ) revert();

        uint256 _indexValue = indexValue;
        uint256 _currentEpoch = currentEpoch;
        address _owner = owner;

        for (uint256 i = 0; i < v.length; i++) {
            bytes32 commit;
            uint256 bTime = _bufferTime;
            if (i < splitIndex){
                // type 1 commits
                commit = keccak256(
                    abi.encodePacked(
                        uint256(index[i]), 
                        uint256(_currentEpoch), 
                        uint256(u[i]), 
                        uint256(1), 
                        type1IAddress
                    )
                );

                burnBufferIAddress[val] = type1IAddress;

                // if owner isn't iAddress
                // add invDisadvantageTime to buffer time
                if (_owner != type1IAddress){
                    bTime += invDisadvantageTime;
                }
            }else {
                // type 2 commit
                // Note that type 2 commits iAddress are 
                // onwers themselves by default
                commit = keccak256(
                    abi.encodePacked(
                        uint256(index[i]), 
                        uint256(_currentEpoch), 
                        uint256(u[i]),  
                        uint256(2), 
                        _onwer, 
                        type2ToAddress
                    )
                );
                burnBufferIAddress[val] = _owner;
            }

            // checks that owner signed the commitment
            if (ecrecover(val, v[i], r[i], s[i]) != _owner) revert();

            burnBufferTime[val] = bTime;
            burnBufferIBlob[val] = keccak256(abi.encodePacked(u[i]));
        }

        // TODO emit event
    }

    function challengeBurnCommits(
        bytes32[] calldata commitHash,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s
    ) public {
        // sanity checks
        if (commitHash.length != v.length) revert();
        if (
            v.length != r.length || 
            v.length != s.length ||
        ) revert();

        for (uint256 i = 0; i < commitHash.length; i++) {
            if (burnBufferTime[hashVals[i]] <= block.timestamp) revert();
            if (ecrecover(burnBufferIBlob[commitHash[i]], v[i], r[i], s[i]) != burnBufferIAddress[commitHash[i]]) revert();

            // challenge is valid, delete records
            delete burnBufferTime[commitHash[i]];
            delete burnBufferIBlob[commitHash[i]];
            delete burnBufferIaddress[commitHash[i]];
        }
    }


    function claimBurnCommits(
        bytes32[] calldata commitHash
    ) public {

    }

} 