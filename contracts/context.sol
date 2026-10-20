// The transaction context, read through __sys_caller, __sys_callvalue,
// __sys_address, __sys_block_number and __sys_timestamp.
contract context {
	address public owner;
	uint256 public deposits;

	constructor() {
		owner = msg.sender;
	}

	function whoami() public view returns (address) {
		return msg.sender;
	}

	function me() public view returns (address) {
		return address(this);
	}

	function when() public view returns (uint256, uint256) {
		return (block.number, block.timestamp);
	}

	function deposit() public payable {
		deposits += msg.value;
	}

	function onlyOwner() public view returns (bool) {
		require(msg.sender == owner);
		return true;
	}
}
